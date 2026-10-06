//! Explicit preservation of descriptors whose backup body is missing.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedDescriptor {
    pub metadata: PathBuf,
    pub original_metadata: PathBuf,
    pub backup: PathBuf,
    pub original: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}
pub(super) fn is_retained(name: &str) -> bool {
    name.strip_suffix(".retained").is_some_and(is_descriptor)
}
pub(super) fn inspect(vault: &Vault, relative: &Path) -> Result<RetainedDescriptor> {
    ensure!(
        relative
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_retained),
        "保留记录名称无效"
    );
    let original_metadata = relative.with_extension("");
    let descriptor = read_descriptor(vault, relative)?;
    ensure!(descriptor.unknown.is_empty(), "保留记录含未知字段");
    let (original, backup) = descriptor_paths(&original_metadata, &descriptor)?;
    // If a body reappears, it needs explicit reconciliation, not silent GC.
    match fs::symlink_metadata(vault.regular_file_path(&backup)?) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => anyhow::bail!("保留记录的正文路径已重新出现"),
    }
    let meta = fs::symlink_metadata(vault.regular_file_path(relative)?)?;
    Ok(RetainedDescriptor {
        metadata: relative.to_owned(),
        original_metadata,
        backup,
        original,
        bytes: meta.len(),
        modified: meta.modified()?,
    })
}

/// Call only after the user reviews the missing-body record and explicitly
/// chooses to retain its metadata separately. Moves exactly those bytes beside
/// the old descriptor, never deletes them or touches notes/backup bodies.
/// Coordination covers cooperating processes of this local user; external
/// writers must be stopped by the caller's workflow.
pub fn retain(vault: &Vault, expected: &MissingPayload) -> Result<RetainedDescriptor> {
    retain_with(vault, expected, || {})
}
fn retain_with(
    vault: &Vault,
    expected: &MissingPayload,
    before_move: impl FnOnce(),
) -> Result<RetainedDescriptor> {
    retain_with_hooks(vault, expected, before_move, || {})
}
fn retain_with_hooks(
    vault: &Vault,
    expected: &MissingPayload,
    before_move: impl FnOnce(),
    after_move: impl FnOnce(),
) -> Result<RetainedDescriptor> {
    crate::vault::backup::storage::require_local(&vault.root)?;
    let _operation = super::super::lock_operation(vault)?;
    let path = vault.regular_file_path(&expected.metadata)?;
    crate::vault::backup::storage::require_local(&path)?;
    let guard = SaveGuard::open(&path)?;
    ensure!(
        missing_payload(vault, &expected.metadata)? == *expected,
        "缺失记录已变化，请刷新后重试"
    );
    let name = format!(
        "{}.retained",
        expected
            .metadata
            .file_name()
            .context("描述路径无效")?
            .to_str()
            .context("描述名称无效")?
    );
    let relative = expected.metadata.with_file_name(name);
    let destination = vault.regular_file_path(&relative)?;
    before_move();
    ensure!(
        guard.matches(&path)? && missing_payload(vault, &expected.metadata)? == *expected,
        "归档前记录或正文状态已变化"
    );
    move_no_replace(&path, &destination)?;
    after_move();
    // No rollback over a newly created source. A post-move failure preserves the
    // moved bytes; inventory will expose an abnormal archive and block cleanup.
    ensure!(
        guard.matches(&destination)?,
        "归档期间描述文件被替换，已保留文件供检查"
    );
    inspect(vault, &relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn fixture() -> Result<(PathBuf, Vault, MissingPayload, Vec<u8>)> {
        let root = std::env::temp_dir().join(format!("inkstone-residue-{}", unique_id()));
        fs::create_dir_all(root.join("vault"))?;
        let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
        fs::write(vault.root.join("note.md"), b"current")?;
        let bytes = serde_json::to_vec(
            &serde_json::json!({"original":"note.md","backup":".inkstone-sync-cleanup-test.backup","sha256":hash(b"gone"),"protected":true}),
        )?;
        fs::write(
            vault.root.join(".inkstone-sync-cleanup-test.backup.json"),
            &bytes,
        )?;
        let missing = inventory(&vault)?.missing_payloads.remove(0);
        Ok((root, vault, missing, bytes))
    }
    #[test]
    fn retention_preserves_protected_bytes_and_accounting_without_touching_notes() -> Result<()> {
        let (root, vault, missing, bytes) = fixture()?;
        let record = retain(&vault, &missing)?;
        assert_eq!(fs::read(vault.root.join(&record.metadata))?, bytes);
        assert!(!vault.root.join(&missing.metadata).exists());
        assert_eq!(fs::read(vault.root.join("note.md"))?, b"current");
        let report = inventory(&vault)?;
        assert_eq!(report.stored_bytes, bytes.len() as u64);
        assert_eq!(report.unreadable, 0);
        assert!(report.missing_payloads.is_empty());
        assert_eq!(report.retained_descriptors, vec![record.clone()]);
        assert!(retain(&vault, &missing).is_err());
        // Reappearing data is protected as unindexed, not silently ignored.
        fs::write(vault.root.join(&missing.backup), b"returned")?;
        let report = inventory(&vault)?;
        assert_eq!(report.unreadable, 1);
        assert_eq!(report.unindexed_files, 1);
        assert_eq!(fs::read(vault.root.join(record.metadata))?, bytes);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn active_sync_lock_refuses_archiving_and_releases_after_success() -> Result<()> {
        let (root, vault, missing, bytes) = fixture()?;
        let lock = super::super::super::lock_operation(&vault)?;
        assert!(retain(&vault, &missing).is_err());
        assert_eq!(fs::read(vault.root.join(&missing.metadata))?, bytes);
        drop(lock);
        retain(&vault, &missing)?;
        drop(super::super::super::lock_operation(&vault)?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn stale_content_existing_archive_and_reappearing_body_all_refuse_move() -> Result<()> {
        for case in 0..4 {
            let (root, vault, missing, bytes) = fixture()?;
            let path = vault.root.join(&missing.metadata);
            let archive = path.with_file_name(format!(
                "{}.retained",
                path.file_name().unwrap().to_str().unwrap()
            ));
            let change_descriptor = || {
                if case == 0 {
                    fs::write(&path, b"{}").unwrap();
                } else {
                    let value = String::from_utf8(bytes.clone())
                        .unwrap()
                        .replace(&hash(b"gone"), &hash(b"other"));
                    fs::write(&path, value).unwrap();
                    fs::File::options()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_times(fs::FileTimes::new().set_modified(missing.modified))
                        .unwrap();
                }
            };
            // Windows denies in-place writes while SaveGuard is open. Test
            // stale content before admission there; Unix also tests the gap.
            #[cfg(windows)]
            if case == 0 || case == 3 {
                change_descriptor();
            }
            let result = retain_with(&vault, &missing, || match case {
                1 => {
                    fs::write(&archive, b"existing").unwrap();
                }
                2 => {
                    fs::write(vault.root.join(&missing.backup), b"returned").unwrap();
                }
                _ => {
                    #[cfg(not(windows))]
                    change_descriptor();
                }
            });
            assert!(result.is_err(), "case {case}");
            assert!(path.exists());
            if case == 1 {
                assert_eq!(fs::read(&archive)?, b"existing");
            }
            assert_eq!(fs::read(vault.root.join("note.md"))?, b"current");
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn windows_guard_denies_descriptor_write_during_retention() -> Result<()> {
        let (root, vault, missing, bytes) = fixture()?;
        let path = vault.root.join(&missing.metadata);
        let record = retain_with(&vault, &missing, || {
            let error = fs::write(&path, b"changed").unwrap_err();
            assert_eq!(error.raw_os_error(), Some(32));
        })?;
        assert_eq!(fs::read(vault.root.join(record.metadata))?, bytes);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn killed_retention_preserves_metadata_and_fresh_process_can_continue() -> Result<()> {
        const ROOT: &str = "INKSTONE_TEST_RETAIN_KILL_ROOT";
        const MODE: &str = "INKSTONE_TEST_RETAIN_KILL_MODE";
        const TEST: &str = "vault::sync::recovery::residue::tests::killed_retention_preserves_metadata_and_fresh_process_can_continue";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
            let mode = std::env::var(MODE)?;
            let mut report = inventory(&vault)?;
            if mode == "resume" {
                if let Some(missing) = report.missing_payloads.pop() {
                    retain(&vault, &missing)?;
                }
                let report = inventory(&vault)?;
                ensure!(
                    report.missing_payloads.is_empty()
                        && report.retained_descriptors.len() == 1
                        && report.unreadable == 0,
                    "unexpected resumed inventory"
                );
                // Reopening in this new process also proves the old process's
                // OS locks did not remain held after forced termination.
                drop(super::super::super::lock_operation(&vault)?);
                fs::write(root.join("resumed"), b"ok")?;
                return Ok(());
            }
            let pause = || {
                fs::write(root.join("checkpoint"), mode.as_bytes()).unwrap();
                loop {
                    std::thread::sleep(Duration::from_millis(50));
                }
            };
            let missing = report.missing_payloads.remove(0);
            retain_with_hooks(
                &vault,
                &missing,
                || {
                    if mode == "before" {
                        pause();
                    }
                },
                || {
                    if mode == "after" {
                        pause();
                    }
                },
            )?;
            anyhow::bail!("child was not killed at checkpoint");
        }
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        fn spawn(root: &Path, mode: &str) -> Result<Child> {
            Ok(Child(
                std::process::Command::new(std::env::current_exe()?)
                    .args(["--exact", TEST])
                    .env(ROOT, root)
                    .env(MODE, mode)
                    .stdout(std::process::Stdio::null())
                    .spawn()?,
            ))
        }
        for mode in ["before", "after"] {
            let (root, vault, missing, bytes) = fixture()?;
            let mut child = spawn(&root, mode)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while !root.join("checkpoint").exists() {
                ensure!(
                    child.0.try_wait()?.is_none(),
                    "child exited before checkpoint"
                );
                ensure!(std::time::Instant::now() < deadline, "checkpoint timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            child.0.kill()?;
            ensure!(!child.0.wait()?.success(), "child not terminated");
            let report = inventory(&vault)?;
            assert_eq!(report.missing_payloads.len(), usize::from(mode == "before"));
            assert_eq!(
                report.retained_descriptors.len(),
                usize::from(mode == "after")
            );
            assert_eq!(report.stored_bytes, bytes.len() as u64);
            let preserved = if mode == "before" {
                missing.metadata.clone()
            } else {
                report.retained_descriptors[0].metadata.clone()
            };
            assert_eq!(fs::read(vault.root.join(preserved))?, bytes);
            let mut resumed = spawn(&root, "resume")?;
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = resumed.0.try_wait()? {
                    ensure!(status.success(), "resume failed");
                    break;
                }
                ensure!(std::time::Instant::now() < deadline, "resume timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(root.join("resumed").exists());
            let report = inventory(&vault)?;
            assert_eq!(report.retained_descriptors.len(), 1);
            assert_eq!(
                fs::read(vault.root.join(&report.retained_descriptors[0].metadata))?,
                bytes
            );
            assert_eq!(fs::read(vault.root.join("note.md"))?, b"current");
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }
}
