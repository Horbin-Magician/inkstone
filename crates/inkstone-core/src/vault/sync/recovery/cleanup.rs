//! Locked preflight; a checked preview is not a persistent deletion authorization.
use super::{retention, *};

pub struct Checked {
    preview: retention::Preview,
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
/// There is deliberately no deletion method yet.
pub fn prepare(
    vault: &Vault,
    expected: &retention::Preview,
    in_use: &BTreeSet<PathBuf>,
    cancellation: &Cancellation,
) -> Result<Checked> {
    cancellation.check()?;
    crate::vault::backup::storage::require_local(&vault.root)?;
    let operation = super::super::lock_operation(vault)?;
    let fresh = inventory(vault)?;
    ensure!(
        fresh.unreadable == 0 && fresh.unindexed_files == 0 && fresh.unmeasured_files == 0,
        "同步备份清单不完整，已保留全部备份"
    );
    ensure!(
        retention::preview(&fresh, expected.keep_per_note, in_use)? == *expected,
        "同步备份或保护状态已变化，请重新预览"
    );
    ensure!(expected.candidates > 0, "没有可核验的清理候选");
    // A corrupt retained newest record must not justify removing a good older one.
    for entry in &fresh.entries {
        cancellation.check()?;
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
            cancellation.check()?;
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
    }
    cancellation.check()?;
    let after = inventory(vault)?;
    ensure!(
        after.unreadable == 0
            && after.unindexed_files == 0
            && after.unmeasured_files == 0
            && retention::preview(&after, expected.keep_per_note, in_use)? == *expected,
        "核验期间同步备份清单发生变化，请重新预览"
    );
    Ok(Checked {
        preview: expected.clone(),
        _operation: operation,
    })
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
