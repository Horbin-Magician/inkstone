//! Metadata-only inventory of displaced local files retained by sync.
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
    fn walk(vault: &Vault, directory: &Path, report: &mut Inventory) -> io::Result<()> {
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
            if is_descriptor(&name) {
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
                        if walk(vault, &path, report).is_err() {
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
    walk(vault, &vault.root, &mut report)?;
    report.entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.backup.cmp(&b.backup))
    });
    Ok(report)
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
        apply(&vault, "note.md", Some(&hash(b"old")), Some(b"new"))?;
        apply(
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
    fn malformed_and_out_of_scope_descriptors_do_not_hide_valid_records() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("inkstone-sync-inventory-bad-{}", unique_id()));
        fs::create_dir_all(root.join("vault/nested"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), "old")?;
        apply(&vault, "note.md", Some(&hash(b"old")), None)?;
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
}
