//! Inventory and no-clobber recovery of displaced local files retained by sync.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Paths are relative to this vault, including the hidden backup filename.
    pub metadata: PathBuf,
    pub backup: PathBuf,
    pub original: PathBuf,
    pub modified: SystemTime,
    pub bytes: u64,
    /// Baseline recorded before sync, not a claim that backup bytes were verified.
    /// A concurrent external edit can make the preserved backup differ from it.
    pub expected_sha256: String,
}
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub entries: Vec<Entry>,
    /// Invalid/unreadable descriptors and directories; valid entries remain usable.
    pub unreadable: usize,
    /// Sum of validated backup file sizes, excluding descriptors and orphan files.
    pub bytes: u64,
    /// Sum of regular backup payload and descriptor lengths; not allocated disk blocks.
    pub stored_bytes: u64,
    /// Regular payloads without a usable recovery record. Never cleanup candidates.
    pub unindexed_files: usize,
    pub unindexed_bytes: u64,
    /// Matching paths whose size could not be measured safely (including links).
    pub unmeasured_files: usize,
}
#[derive(Deserialize)]
struct Descriptor {
    original: String,
    backup: String,
    sha256: String,
}
fn is_descriptor(name: &str) -> bool {
    name.starts_with(".inkstone-sync-") && name.ends_with(".backup.json")
}
fn inspect(vault: &Vault, relative: &Path) -> Result<Entry> {
    let path = vault.regular_file_path(relative)?;
    let descriptor_meta = fs::symlink_metadata(&path)?;
    ensure!(
        descriptor_meta.is_file() && !is_reparse(&descriptor_meta),
        "备份描述文件类型无效"
    );
    ensure!(descriptor_meta.len() <= 16 * 1024, "备份描述文件过大");
    ensure!(
        relative
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(is_descriptor),
        "备份描述文件名称无效"
    );
    let descriptor: Descriptor = serde_json::from_slice(&fs::read(&path)?)?;
    validate_path(&descriptor.original)?;
    ensure!(valid_hash(&descriptor.sha256), "备份校验信息无效");
    let original = PathBuf::from(&descriptor.original);
    let parent = relative.parent().context("备份描述路径无效")?;
    ensure!(original.parent() == Some(parent), "备份不在原文件目录");
    // The descriptor names exactly its adjacent payload, never another file.
    let backup = relative.with_extension("");
    ensure!(
        backup.file_name().and_then(|s| s.to_str()) == Some(descriptor.backup.as_str()),
        "备份文件名不匹配"
    );
    let backup_path = vault.regular_file_path(&backup)?;
    let meta = fs::symlink_metadata(backup_path)?;
    ensure!(meta.is_file() && !is_reparse(&meta), "备份文件类型无效");
    Ok(Entry {
        metadata: relative.to_owned(),
        backup,
        original,
        modified: meta.modified()?,
        bytes: meta.len(),
        expected_sha256: descriptor.sha256,
    })
}

/// Enumerate metadata without reading payloads or changing notes/backups.
/// Hidden directories are excluded consistently with the sync file scope.
pub fn inventory(vault: &Vault) -> Result<Inventory> {
    fn walk(
        vault: &Vault,
        directory: &Path,
        report: &mut Inventory,
        payloads: &mut BTreeMap<PathBuf, u64>,
    ) -> io::Result<()> {
        for item in fs::read_dir(directory)? {
            let item = match item {
                Ok(item) => item,
                Err(_) => {
                    report.unreadable += 1;
                    continue;
                }
            };
            let name = item.file_name();
            let name = name.to_string_lossy();
            let path = item.path();
            let descriptor = is_descriptor(&name);
            let payload = name.starts_with(".inkstone-sync-") && name.ends_with(".backup");
            if descriptor || payload {
                match fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_file() && !is_reparse(&meta) => {
                        report.stored_bytes = report.stored_bytes.saturating_add(meta.len());
                        if payload {
                            payloads.insert(
                                path.strip_prefix(&vault.root).unwrap().to_owned(),
                                meta.len(),
                            );
                        }
                    }
                    _ => report.unmeasured_files += 1,
                }
            }
            if descriptor {
                match path
                    .strip_prefix(&vault.root)
                    .ok()
                    .and_then(|p| inspect(vault, p).ok())
                {
                    Some(entry) => {
                        report.bytes = report.bytes.saturating_add(entry.bytes);
                        report.entries.push(entry);
                    }
                    None => report.unreadable += 1,
                }
            } else if !name.starts_with('.') {
                match fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_dir() && !is_reparse(&meta) => {
                        if walk(vault, &path, report, payloads).is_err() {
                            report.unreadable += 1;
                        }
                    }
                    Err(_) => report.unreadable += 1,
                    _ => {}
                }
            }
        }
        Ok(())
    }
    let mut report = Inventory::default();
    let mut payloads = BTreeMap::new();
    walk(vault, &vault.root, &mut report, &mut payloads)?;
    for entry in &report.entries {
        payloads.remove(&entry.backup);
    }
    report.unindexed_files = payloads.len();
    report.unindexed_bytes = payloads
        .values()
        .fold(0u64, |sum, bytes| sum.saturating_add(*bytes));
    report.entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.backup.cmp(&b.backup))
    });
    Ok(report)
}

#[derive(Clone, Debug)]
pub struct Restored {
    pub relative: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    /// False can indicate an external write preserved by the sync race guard.
    pub matches_baseline: bool,
}

/// Restore the reviewed backup as a new sibling file, retaining both the backup
/// and any current original. `reserved` includes unsaved/open editor paths.
pub fn restore_copy(vault: &Vault, entry: &Entry, reserved: &[PathBuf]) -> Result<Restored> {
    restore_with(vault, entry, reserved, || {})
}
fn restore_with(
    vault: &Vault,
    entry: &Entry,
    reserved: &[PathBuf],
    copied: impl FnOnce(),
) -> Result<Restored> {
    ensure!(
        inspect(vault, &entry.metadata)? == *entry,
        "备份记录已变化，请刷新后重试"
    );
    let backup = vault.regular_file_path(&entry.backup)?;
    let mut source = SaveGuard::open(&backup)?;
    let matches = |meta: &fs::Metadata| {
        meta.len() == entry.bytes && meta.modified().ok() == Some(entry.modified)
    };
    ensure!(
        matches(&source.0.metadata()?),
        "备份内容已变化，请刷新后重试"
    );
    let parent = entry.original.parent().context("备份原路径无效")?;
    let staged = vault
        .regular_file_path(&parent.join(format!(".inkstone-sync-{}.restore-tmp", unique_id())))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let result = (|| -> Result<Restored> {
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = source.0.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            ensure!(
                (count as u64) <= entry.bytes.saturating_sub(total),
                "恢复期间备份大小增加，请刷新后重试"
            );
            output.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            total += count as u64;
        }
        #[cfg(unix)]
        fs::set_permissions(&staged, source.0.metadata()?.permissions())?;
        output.sync_all()?;
        drop(output);
        copied();
        ensure!(
            total == entry.bytes
                && matches(&source.0.metadata()?)
                && source.matches(&backup)?
                && inspect(vault, &entry.metadata)? == *entry,
            "恢复期间备份记录已变化，请刷新后重试"
        );
        let sha256 = format!("{:x}", digest.finalize());
        let stem = entry
            .original
            .file_stem()
            .context("备份原路径无效")?
            .to_string_lossy();
        let extension = entry
            .original
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        for serial in 1u64.. {
            let suffix = if serial == 1 {
                String::new()
            } else {
                format!(" {serial}")
            };
            let relative = parent.join(format!("{stem} 同步恢复{suffix}{extension}"));
            if reserved.iter().any(|p| {
                p.to_string_lossy().replace('\\', "/").to_lowercase()
                    == relative.to_string_lossy().replace('\\', "/").to_lowercase()
            }) {
                continue;
            }
            let destination = vault.regular_file_path(&relative)?;
            // Publish only complete content. Existing files and links are never replaced.
            match fs::hard_link(&staged, &destination) {
                Ok(()) => {
                    return Ok(Restored {
                        relative,
                        bytes: total,
                        matches_baseline: sha256 == entry.expected_sha256,
                        sha256,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    })();
    let _ = fs::remove_file(&staged);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_includes_replacements_deletions_and_binary_files_without_mutation() -> Result<()> {
        let root = std::env::temp_dir().join(format!("inkstone-sync-inventory-{}", unique_id()));
        fs::create_dir_all(root.join("vault/nested"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), "old")?;
        fs::write(vault.root.join("nested/image.bin"), [0, 255, 1, 2])?;
        apply_bytes(&vault, "note.md", Some(&hash(b"old")), Some(b"new"))?;
        apply_bytes(
            &vault,
            "nested/image.bin",
            Some(&hash(&[0, 255, 1, 2])),
            None,
        )?;
        let report = inventory(&vault)?;
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.bytes, 7);
        assert_eq!(report.unreadable, 0);
        assert_eq!(fs::read(vault.root.join("note.md"))?, b"new");
        assert!(!vault.root.join("nested/image.bin").exists());
        for entry in &report.entries {
            let bytes = fs::read(vault.root.join(&entry.backup))?;
            assert_eq!(hash(&bytes), entry.expected_sha256);
        }
        assert_eq!(inventory(&vault)?.entries, report.entries);
        // Listing remains metadata-only even when payload no longer matches its baseline.
        let payload = vault.root.join(&report.entries[0].backup);
        OpenOptions::new()
            .write(true)
            .open(&payload)?
            .set_len(64 * 1024 * 1024)?;
        assert!(
            inventory(&vault)?
                .entries
                .iter()
                .any(|e| e.bytes == 64 * 1024 * 1024)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn capacity_counts_unindexed_backups_without_reading_or_removing_them() -> Result<()> {
        let root = std::env::temp_dir().join(format!("inkstone-sync-capacity-{}", unique_id()));
        fs::create_dir_all(root.join("vault/nested"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), b"old")?;
        apply_bytes(&vault, "note.md", Some(&hash(b"old")), Some(b"new"))?;
        let initial = inventory(&vault)?;
        let entry = &initial.entries[0];
        let descriptor_bytes = fs::metadata(vault.root.join(&entry.metadata))?.len();
        assert_eq!(initial.stored_bytes, 3 + descriptor_bytes);
        assert_eq!(initial.unindexed_files, 0);
        let orphan = vault.root.join("nested/.inkstone-sync-orphan.backup");
        fs::write(&orphan, b"orphan")?;
        let broken = vault.root.join(".inkstone-sync-broken.backup");
        fs::write(&broken, b"protected")?;
        let descriptor = vault.root.join(".inkstone-sync-broken.backup.json");
        fs::write(&descriptor, b"invalid")?;
        // A large sparse payload ensures accounting needs metadata, not a valid body.
        OpenOptions::new()
            .write(true)
            .open(&orphan)?
            .set_len(64 * 1024 * 1024)?;
        fs::write(vault.root.join("ordinary.backup"), b"not a sync backup")?;
        let report = inventory(&vault)?;
        assert_eq!(report.entries, initial.entries);
        assert_eq!(report.bytes, 3);
        assert_eq!(report.unindexed_files, 2);
        assert_eq!(report.unindexed_bytes, 64 * 1024 * 1024 + 9);
        assert_eq!(
            report.stored_bytes,
            initial.stored_bytes + report.unindexed_bytes + 7
        );
        assert_eq!(report.unreadable, 1);
        assert_eq!(report.unmeasured_files, 0);
        assert_eq!(fs::read(&broken)?, b"protected");
        assert_eq!(fs::read(&descriptor)?, b"invalid");
        assert_eq!(fs::metadata(&orphan)?.len(), 64 * 1024 * 1024);
        #[cfg(unix)]
        {
            let outside = root.join("outside");
            fs::write(&outside, b"outside")?;
            std::os::unix::fs::symlink(&outside, vault.root.join(".inkstone-sync-link.backup"))?;
            let with_link = inventory(&vault)?;
            assert_eq!(with_link.stored_bytes, report.stored_bytes);
            assert_eq!(with_link.unindexed_files, 2);
            assert_eq!(with_link.unmeasured_files, 1);
            assert_eq!(fs::read(outside)?, b"outside");
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn malformed_and_out_of_scope_descriptors_do_not_hide_valid_records() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("inkstone-sync-inventory-bad-{}", unique_id()));
        fs::create_dir_all(root.join("vault/nested"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), "old")?;
        apply_bytes(&vault, "note.md", Some(&hash(b"old")), None)?;
        for (id, original, backup) in [
            (
                "traversal",
                "../outside.md",
                ".inkstone-sync-traversal.backup",
            ),
            ("parent", "nested/note.md", ".inkstone-sync-parent.backup"),
            ("name", "note.md", "note.md"),
        ] {
            fs::write(
                vault.root.join(format!(".inkstone-sync-{id}.backup.json")),
                serde_json::to_vec(
                    &serde_json::json!({"original":original,"backup":backup,"sha256":hash(b"old")}),
                )?,
            )?;
        }
        fs::write(
            vault.root.join(".inkstone-sync-broken.backup.json"),
            b"broken",
        )?;
        let report = inventory(&vault)?;
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.unreadable, 4);
        #[cfg(unix)]
        {
            let entry = &report.entries[0];
            fs::remove_file(vault.root.join(&entry.backup))?;
            fs::write(root.join("outside"), "outside")?;
            std::os::unix::fs::symlink(root.join("outside"), vault.root.join(&entry.backup))?;
            let report = inventory(&vault)?;
            assert!(report.entries.is_empty());
            assert_eq!(report.unreadable, 5);
            assert_eq!(fs::read(root.join("outside"))?, b"outside");
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn restore_copy_preserves_binary_bytes_existing_files_and_reserved_paths() -> Result<()> {
        let root = std::env::temp_dir().join(format!("inkstone-sync-restore-{}", unique_id()));
        fs::create_dir_all(root.join("vault"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        let bytes = [0, 255, 1, 128];
        fs::write(vault.root.join("image.bin"), bytes)?;
        apply_bytes(&vault, "image.bin", Some(&hash(&bytes)), Some(b"current"))?;
        let entry = inventory(&vault)?.entries.remove(0);
        fs::write(vault.root.join("image 同步恢复.bin"), b"existing")?;
        let restored = restore_copy(&vault, &entry, &[PathBuf::from("image 同步恢复 2.bin")])?;
        assert_eq!(restored.relative, Path::new("image 同步恢复 3.bin"));
        assert_eq!(restored.bytes, 4);
        assert!(restored.matches_baseline);
        assert_eq!(restored.sha256, hash(&bytes));
        assert_eq!(fs::read(vault.root.join(&restored.relative))?, bytes);
        assert_eq!(fs::read(vault.root.join("image.bin"))?, b"current");
        assert_eq!(
            fs::read(vault.root.join("image 同步恢复.bin"))?,
            b"existing"
        );
        assert_eq!(fs::read(vault.root.join(&entry.backup))?, bytes);
        assert!(vault.root.join(&entry.metadata).is_file());
        // A retained concurrent write remains recoverable, explicitly reported as different.
        fs::write(vault.root.join(&entry.backup), b"late external edit")?;
        let changed = inventory(&vault)?.entries.remove(0);
        let restored = restore_copy(&vault, &changed, &[])?;
        assert!(!restored.matches_baseline);
        assert_eq!(
            fs::read(vault.root.join(restored.relative))?,
            b"late external edit"
        );
        assert!(restore_copy(&vault, &entry, &[]).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn changing_backup_during_copy_does_not_publish_partial_recovery() -> Result<()> {
        let root = std::env::temp_dir().join(format!("inkstone-sync-restore-race-{}", unique_id()));
        fs::create_dir_all(root.join("vault"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), b"original")?;
        apply_bytes(&vault, "note.md", Some(&hash(b"original")), None)?;
        let entry = inventory(&vault)?.entries.remove(0);
        assert!(
            restore_with(&vault, &entry, &[], || {
                fs::rename(
                    vault.root.join(&entry.backup),
                    vault.root.join("held.backup"),
                )
                .unwrap();
                fs::write(vault.root.join(&entry.backup), b"changed while restoring").unwrap();
            })
            .is_err()
        );
        assert!(!vault.root.join("note 同步恢复.md").exists());
        assert!(!fs::read_dir(&vault.root)?.any(|e| {
            e.unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "restore-tmp")
        }));
        assert_eq!(
            fs::read(vault.root.join(&entry.backup))?,
            b"changed while restoring"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
