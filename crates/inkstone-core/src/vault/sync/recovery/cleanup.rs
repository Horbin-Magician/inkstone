//! Locked preflight; a checked preview is not a persistent deletion authorization.
use super::{retention, *};

#[derive(Debug)]
struct CleanupCancelled;
impl std::fmt::Display for CleanupCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("同步备份清理已停止；此前已完成的删除不会撤销")
    }
}
impl std::error::Error for CleanupCancelled {}
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.is::<CleanupCancelled>()
}
fn check_cancellation(cancellation: &Cancellation) -> Result<()> {
    if cancellation.is_requested() {
        return Err(CleanupCancelled.into());
    }
    Ok(())
}

pub struct Checked {
    preview: retention::Preview,
    vault: Vault,
    in_use: BTreeSet<PathBuf>,
    _operation: super::super::OperationLock,
}
impl Checked {
    pub fn preview(&self) -> &retention::Preview {
        &self.preview
    }
}

/// Re-read the complete inventory and verify all retained and candidate bodies.
/// Any corruption, unknown record, stale protection or cancellation preserves all
/// files. The returned plan keeps the sync/recovery lock until dropped.
/// Execution revalidates this snapshot; dropping it never changes files.
pub fn prepare(
    vault: &Vault,
    expected: &retention::Preview,
    in_use: &BTreeSet<PathBuf>,
    cancellation: &Cancellation,
) -> Result<Checked> {
    check_cancellation(cancellation)?;
    crate::vault::backup::storage::require_local(&vault.root)?;
    let operation = super::super::lock_operation(vault)?;
    validate(vault, expected, in_use, cancellation)?;
    Ok(Checked {
        preview: expected.clone(),
        vault: vault.clone(),
        in_use: in_use.clone(),
        _operation: operation,
    })
}

fn verify_entry(vault: &Vault, entry: &Entry, cancellation: &Cancellation) -> Result<()> {
    check_cancellation(cancellation)?;
    ensure!(inspect(vault, &entry.metadata)? == *entry, "备份记录已变化");
    let descriptor = read_descriptor(vault, &entry.metadata)?;
    ensure!(
        descriptor.unknown.is_empty(),
        "备份包含未知格式字段，已保留全部备份"
    );
    let path = vault.regular_file_path(&entry.backup)?;
    crate::vault::backup::storage::require_local(&path)?;
    let mut source = SaveGuard::open(&path)?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        check_cancellation(cancellation)?;
        let count = source.0.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        ensure!(
            (count as u64) <= entry.bytes.saturating_sub(bytes),
            "核验期间备份变大"
        );
        bytes += count as u64;
        digest.update(&buffer[..count]);
    }
    ensure!(
        bytes == entry.bytes && format!("{:x}", digest.finalize()) == entry.expected_sha256,
        "备份正文与同步前基线不同，已保留全部备份"
    );
    ensure!(source.matches(&path)?, "核验期间备份文件被替换");
    ensure!(
        inspect(vault, &entry.metadata)? == *entry,
        "核验期间备份记录已变化"
    );
    Ok(())
}

fn verify_inventory(vault: &Vault, expected: &[Entry], cancellation: &Cancellation) -> Result<()> {
    check_cancellation(cancellation)?;
    crate::vault::backup::storage::require_local(&vault.root)?;
    let fresh = inventory(vault)?;
    ensure!(
        fresh.unreadable == 0 && fresh.unindexed_files == 0 && fresh.unmeasured_files == 0,
        "同步备份清单不完整，已保留剩余备份"
    );
    ensure!(
        fresh.entries.len() == expected.len()
            && fresh.entries.iter().all(|entry| expected.contains(entry)),
        "同步备份或保护状态已变化，请重新预览"
    );
    // Verify retained records too: a corrupt newest copy cannot justify deletion.
    for entry in expected {
        verify_entry(vault, entry, cancellation)?;
    }
    let after = inventory(vault)?;
    ensure!(
        after.unreadable == 0
            && after.unindexed_files == 0
            && after.unmeasured_files == 0
            && after.entries == fresh.entries,
        "核验期间同步备份清单发生变化，请重新预览"
    );
    check_cancellation(cancellation)?;
    Ok(())
}

fn validate(
    vault: &Vault,
    expected: &retention::Preview,
    in_use: &BTreeSet<PathBuf>,
    cancellation: &Cancellation,
) -> Result<()> {
    ensure!(expected.candidates > 0, "没有可核验的清理候选");
    let fresh = inventory(vault)?;
    ensure!(
        retention::preview(&fresh, expected.keep_per_note, in_use)? == *expected,
        "同步备份或保护状态已变化，请重新预览"
    );
    verify_inventory(vault, &fresh.entries, cancellation)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub removed: usize,
    /// Payload lengths, excluding descriptors and filesystem allocation overhead.
    pub bytes: u64,
}

impl Checked {
    /// Delete only reviewed candidates. Cancellation/errors may leave an already
    /// completed prefix deleted; a displaced pending body remains a protected
    /// recovery record. The caller must refresh inventory even on failure.
    /// Coordination covers cooperating processes of this local user only.
    pub fn execute(self, cancellation: &Cancellation) -> Result<Report> {
        self.execute_with(cancellation, |_| Ok(()))
    }

    fn execute_with(
        self,
        cancellation: &Cancellation,
        mut after_move: impl FnMut(&Entry) -> Result<()>,
    ) -> Result<Report> {
        let vault = &self.vault;
        validate(vault, &self.preview, &self.in_use, cancellation)?;
        let mut remaining: Vec<Entry> = self
            .preview
            .records
            .iter()
            .map(|record| record.backup.clone())
            .collect();
        let mut report = Report::default();
        for record in &self.preview.records {
            if record.decision != retention::Decision::Candidate {
                continue;
            }
            verify_inventory(vault, &remaining, cancellation)?;
            let entry = &record.backup;
            let source = vault.regular_file_path(&entry.backup)?;
            let source_guard = SaveGuard::open(&source)?;
            let metadata = vault.regular_file_path(&entry.metadata)?;
            let metadata_guard = SaveGuard::open(&metadata)?;
            let descriptor = read_descriptor(vault, &entry.metadata)?;
            ensure!(
                descriptor.unknown.is_empty() && !descriptor.protected,
                "备份保护状态已变化"
            );
            let name = format!(".inkstone-sync-cleanup-{}.backup", unique_id());
            let relative = entry.backup.with_file_name(&name);
            let mut displaced = entry.clone();
            displaced.backup = relative.clone();
            displaced.metadata = relative.with_file_name(format!("{name}.json"));
            displaced.protected = true;
            let destination = vault.root.join(&relative);
            let destination_metadata = vault.root.join(&displaced.metadata);
            // Publish recovery instructions before moving bytes. A crash in between
            // leaves the original usable and an unreadable descriptor blocking GC.
            write_new_synced(
                &destination_metadata,
                &serde_json::to_vec(&serde_json::json!({
                    "original": descriptor.original, "backup": name,
                    "sha256": descriptor.sha256, "protected": true
                }))?,
            )?;
            move_no_replace(&source, &destination)?;
            ensure!(
                source_guard.matches(&destination)?,
                "移动期间备份被替换，已保留恢复记录"
            );
            // Compare the complete descriptor as well as its file identity.
            let current = read_descriptor(vault, &entry.metadata)?;
            ensure!(
                metadata_guard.matches(&metadata)?
                    && current.unknown.is_empty()
                    && !current.protected
                    && current.original == descriptor.original
                    && current.backup == descriptor.backup
                    && current.sha256 == descriptor.sha256,
                "移动期间描述记录已变化，已保留恢复记录"
            );
            fs::remove_file(&metadata)?;
            let index = remaining
                .iter()
                .position(|item| item == entry)
                .context("候选记录缺失")?;
            remaining[index] = displaced.clone();
            after_move(&displaced)?;
            verify_inventory(vault, &remaining, cancellation)?;
            ensure!(source_guard.matches(&destination)?, "清理前备份被替换");
            // No cancellation gap between payload and descriptor removal. A crash
            // after payload deletion leaves harmless metadata and blocks later GC.
            fs::remove_file(&destination)?;
            fs::remove_file(&destination_metadata)?;
            remaining.remove(index);
            report.removed += 1;
            report.bytes += entry.bytes;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fixture() -> (PathBuf, Vault, retention::Preview) {
        let root = std::env::temp_dir().join(format!("inkstone-sync-preflight-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("note.md"), b"current").unwrap();
        for (id, body, time) in [("old", "old", 1), ("new", "new", 2)] {
            let name = format!(".inkstone-sync-{id}.backup");
            let path = vault.root.join(&name);
            fs::write(&path, body).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(time)),
                )
                .unwrap();
            fs::write(vault.root.join(format!("{name}.json")), serde_json::to_vec(
                &serde_json::json!({"original":"note.md","backup":name,"sha256":hash(body.as_bytes())})
            ).unwrap()).unwrap();
        }
        let preview = retention::preview(&inventory(&vault).unwrap(), 1, &BTreeSet::new()).unwrap();
        (root, vault, preview)
    }
    #[test]
    fn execution_removes_only_reviewed_candidates_and_releases_lock() {
        let (root, vault, preview) = fixture();
        let cancellation = Cancellation::default();
        let checked = prepare(&vault, &preview, &BTreeSet::new(), &cancellation).unwrap();
        let report = checked
            .execute_with(&cancellation, |displaced| {
                assert!(displaced.protected);
                assert!(super::super::super::lock_operation(&vault).is_err());
                assert_eq!(
                    fs::read(vault.root.join(&displaced.backup)).unwrap(),
                    b"old"
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(
            report,
            Report {
                removed: 1,
                bytes: 3
            }
        );
        let fresh = inventory(&vault).unwrap();
        assert_eq!(fresh.entries.len(), 1);
        assert_eq!(
            fresh.unreadable + fresh.unindexed_files + fresh.unmeasured_files,
            0
        );
        assert_eq!(
            fs::read(vault.root.join(".inkstone-sync-new.backup")).unwrap(),
            b"new"
        );
        assert_eq!(fs::read(vault.root.join("note.md")).unwrap(), b"current");
        assert!(!vault.root.join(".inkstone-sync-old.backup.json").exists());
        drop(super::super::super::lock_operation(&vault).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_or_interruption_after_move_preserves_recoverable_protected_body() {
        for cancel in [false, true] {
            let (root, vault, preview) = fixture();
            let cancellation = Cancellation::default();
            let checked = prepare(&vault, &preview, &BTreeSet::new(), &cancellation).unwrap();
            let error = checked
                .execute_with(&cancellation, |_| {
                    if cancel {
                        cancellation.request();
                        Ok(())
                    } else {
                        anyhow::bail!("injected interruption")
                    }
                })
                .unwrap_err();
            assert_eq!(is_cancelled(&error), cancel);
            if cancel {
                assert!(error.to_string().contains("已完成的删除不会撤销"));
                assert!(!error.to_string().contains("未发布清单或修改本地文件"));
            }
            let fresh = inventory(&vault).unwrap();
            assert_eq!(fresh.entries.len(), 2);
            assert_eq!(
                fresh.unreadable + fresh.unindexed_files + fresh.unmeasured_files,
                0
            );
            let protected = fresh.entries.iter().find(|entry| entry.protected).unwrap();
            assert_eq!(
                retention::preview(&fresh, 1, &BTreeSet::new())
                    .unwrap()
                    .candidates,
                0
            );
            let restored = restore_copy(&vault, protected, &[]).unwrap();
            assert_eq!(
                fs::read(vault.root.join(restored.relative)).unwrap(),
                b"old"
            );
            assert_eq!(fs::read(vault.root.join("note.md")).unwrap(), b"current");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn interrupted_batch_keeps_latest_and_remaining_protected_copy() {
        let (root, vault, _) = fixture();
        let name = ".inkstone-sync-older.backup";
        fs::write(vault.root.join(name), b"older").unwrap();
        fs::File::options()
            .write(true)
            .open(vault.root.join(name))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
        fs::write(
            vault.root.join(format!("{name}.json")),
            serde_json::to_vec(
                &serde_json::json!({"original":"note.md","backup":name,"sha256":hash(b"older")}),
            )
            .unwrap(),
        )
        .unwrap();
        let preview = retention::preview(&inventory(&vault).unwrap(), 1, &BTreeSet::new()).unwrap();
        assert_eq!(preview.candidates, 2);
        let cancellation = Cancellation::default();
        let checked = prepare(&vault, &preview, &BTreeSet::new(), &cancellation).unwrap();
        let mut moved = 0;
        assert!(
            checked
                .execute_with(&cancellation, |_| {
                    moved += 1;
                    if moved == 2 {
                        anyhow::bail!("injected second-record interruption");
                    }
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(moved, 2);
        let fresh = inventory(&vault).unwrap();
        assert_eq!(fresh.entries.len(), 2);
        assert_eq!(
            fresh.unreadable + fresh.unindexed_files + fresh.unmeasured_files,
            0
        );
        assert_eq!(
            fs::read(vault.root.join(".inkstone-sync-new.backup")).unwrap(),
            b"new"
        );
        let protected = fresh.entries.iter().find(|entry| entry.protected).unwrap();
        let restored = restore_copy(&vault, protected, &[]).unwrap();
        assert_eq!(
            fs::read(vault.root.join(restored.relative)).unwrap(),
            b"older"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn execution_rechecks_prepared_snapshot_before_any_mutation() {
        for cancel in [false, true] {
            let (root, vault, preview) = fixture();
            let cancellation = Cancellation::default();
            let checked = prepare(&vault, &preview, &BTreeSet::new(), &cancellation).unwrap();
            if cancel {
                cancellation.request();
            } else {
                fs::write(vault.root.join(".inkstone-sync-new.backup"), b"BAD").unwrap();
            }
            assert!(checked.execute(&cancellation).is_err());
            assert_eq!(
                fs::read(vault.root.join(".inkstone-sync-old.backup")).unwrap(),
                b"old"
            );
            assert_eq!(inventory(&vault).unwrap().entries.len(), 2);
            assert!(
                inventory(&vault)
                    .unwrap()
                    .entries
                    .iter()
                    .all(|e| !e.protected)
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn changes_after_move_stop_deletion_of_displaced_body() {
        for change in ["payload", "retained", "descriptor"] {
            let (root, vault, preview) = fixture();
            let cancellation = Cancellation::default();
            let checked = prepare(&vault, &preview, &BTreeSet::new(), &cancellation).unwrap();
            assert!(
                checked
                    .execute_with(&cancellation, |entry| {
                        let relative = match change {
                            "payload" => entry.backup.clone(),
                            "retained" => PathBuf::from(".inkstone-sync-new.backup"),
                            _ => entry.metadata.clone(),
                        };
                        let path = vault.root.join(relative);
                        // Replace instead of write through the Windows read guard.
                        let time = fs::metadata(&path)?.modified()?;
                        fs::rename(&path, root.join("displaced-original"))?;
                        if change == "descriptor" {
                            let mut value: serde_json::Value = serde_json::from_slice(&fs::read(
                                root.join("displaced-original"),
                            )?)?;
                            value["future_policy"] = serde_json::json!(true);
                            fs::write(&path, serde_json::to_vec(&value)?)?;
                        } else {
                            fs::write(&path, b"BAD")?;
                        }
                        fs::File::options()
                            .write(true)
                            .open(path)?
                            .set_times(fs::FileTimes::new().set_modified(time))?;
                        Ok(())
                    })
                    .is_err()
            );
            let fresh = inventory(&vault).unwrap();
            assert_eq!(fresh.entries.len(), 2);
            let entry = fresh.entries.iter().find(|e| e.protected).unwrap();
            assert_eq!(
                fs::read(vault.root.join(&entry.backup)).unwrap(),
                if change == "payload" { b"BAD" } else { b"old" }
            );
            assert_eq!(fs::read(vault.root.join("note.md")).unwrap(), b"current");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn checked_plan_holds_operation_lock_without_mutating_any_backup() {
        let (root, vault, preview) = fixture();
        let checked =
            prepare(&vault, &preview, &BTreeSet::new(), &Cancellation::default()).unwrap();
        assert_eq!(checked.preview(), &preview);
        assert!(super::super::super::lock_operation(&vault).is_err());
        assert_eq!(fs::read(vault.root.join("note.md")).unwrap(), b"current");
        assert_eq!(
            fs::read(vault.root.join(".inkstone-sync-old.backup")).unwrap(),
            b"old"
        );
        drop(checked);
        drop(super::super::super::lock_operation(&vault).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn same_size_same_time_corruption_of_candidate_or_retained_backup_blocks_preflight() {
        for id in ["old", "new"] {
            let (root, vault, preview) = fixture();
            let path = vault.root.join(format!(".inkstone-sync-{id}.backup"));
            let time = fs::metadata(&path).unwrap().modified().unwrap();
            fs::write(&path, b"BAD").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(time))
                .unwrap();
            assert!(prepare(&vault, &preview, &BTreeSet::new(), &Cancellation::default()).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"BAD");
            assert_eq!(inventory(&vault).unwrap().entries.len(), 2);
            drop(super::super::super::lock_operation(&vault).unwrap());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn unknown_metadata_protects_records_but_keeps_copy_recovery_available() {
        let (root, vault, before) = fixture();
        let path = vault.root.join(".inkstone-sync-old.backup.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["future_recovery_policy"] = serde_json::json!({"keep":true});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let records = inventory(&vault).unwrap();
        assert_eq!(records.unreadable, 0);
        let entry = records
            .entries
            .iter()
            .find(|e| e.backup.ends_with(".inkstone-sync-old.backup"))
            .unwrap();
        assert!(entry.protected);
        let fresh = retention::preview(&records, 1, &BTreeSet::new()).unwrap();
        assert_eq!(fresh.candidates, 0);
        assert!(prepare(&vault, &before, &BTreeSet::new(), &Cancellation::default()).is_err());
        let restored = restore_copy(&vault, entry, &[]).unwrap();
        assert_eq!(
            fs::read(vault.root.join(restored.relative)).unwrap(),
            b"old"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
            value
        );
        // Unknown fields on a retained newest record also block an otherwise valid batch.
        value
            .as_object_mut()
            .unwrap()
            .remove("future_recovery_policy");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let newest = vault.root.join(".inkstone-sync-new.backup.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&newest).unwrap()).unwrap();
        value["format_version"] = serde_json::json!(99);
        fs::write(&newest, serde_json::to_vec(&value).unwrap()).unwrap();
        let fresh = retention::preview(&inventory(&vault).unwrap(), 1, &BTreeSet::new()).unwrap();
        assert_eq!(fresh.candidates, 1);
        assert!(prepare(&vault, &fresh, &BTreeSet::new(), &Cancellation::default()).is_err());
        assert_eq!(fs::read(vault.root.join("note.md")).unwrap(), b"current");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_protection_unknown_backups_and_cancellation_never_authorize_cleanup() {
        let (root, vault, preview) = fixture();
        let protected = BTreeSet::from([PathBuf::from(".inkstone-sync-old.backup")]);
        assert!(prepare(&vault, &preview, &protected, &Cancellation::default()).is_err());
        let cancellation = Cancellation::default();
        cancellation.request();
        assert!(prepare(&vault, &preview, &BTreeSet::new(), &cancellation).is_err());
        fs::write(
            vault.root.join(".inkstone-sync-orphan.backup"),
            b"protected",
        )
        .unwrap();
        assert!(prepare(&vault, &preview, &BTreeSet::new(), &Cancellation::default()).is_err());
        assert_eq!(
            fs::read(vault.root.join(".inkstone-sync-orphan.backup")).unwrap(),
            b"protected"
        );
        assert_eq!(inventory(&vault).unwrap().entries.len(), 2);
        drop(super::super::super::lock_operation(&vault).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
