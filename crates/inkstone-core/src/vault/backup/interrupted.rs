//! Salvage verified, manifest-listed files without changing an interrupted cleanup.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Verified,
    Missing,
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub manifest: Manifest,
    /// One state per manifest file, in the same order. Unlisted files stay untouched.
    pub files: Vec<State>,
}

fn validate_selection(directory: &Path, selected: &Path) -> io::Result<()> {
    if selected.parent() != Some(directory)
        || !selected.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with(".inkstone-backup-cleanup-")
        })
    {
        return Err(invalid("无效的清理中断记录"));
    }
    Ok(())
}

fn inspect_unlocked(selected: &Path) -> io::Result<Preview> {
    let manifest = read_manifest(selected)?;
    let files = manifest
        .files
        .iter()
        .map(|entry| {
            let result = safe_path(selected, &Path::new("files").join(&entry.path))
                .and_then(|path| transfer(&path, None));
            match result {
                Ok((bytes, hash)) if bytes == entry.bytes && hash == entry.sha256 => {
                    State::Verified
                }
                Ok(_) => State::Unavailable("内容校验失败".into()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => State::Missing,
                Err(error) => State::Unavailable(error.to_string()),
            }
        })
        .collect();
    Ok(Preview { manifest, files })
}

/// This is partial recovery information, never a complete-backup integrity verdict.
pub fn inspect(directory: &Path, selected: &Path) -> io::Result<Preview> {
    validate_selection(directory, selected)?;
    let _lock = locking::acquire(directory, false)?;
    inspect_unlocked(selected)
}

/// Publish only verified files to a fresh directory after rechecking the preview.
/// Missing/corrupt/unlisted data and the source manifest remain untouched. This
/// does not clear the interrupted record or permit subsequent retention cleanup.
pub fn restore_verified(
    directory: &Path,
    selected: &Path,
    destination: &Path,
    expected: &Preview,
) -> io::Result<usize> {
    validate_selection(directory, selected)?;
    let _lock = locking::acquire(directory, false)?;
    let preview = inspect_unlocked(selected)?;
    if &preview != expected {
        return Err(invalid("中断记录在预览后发生变化，请重新校验"));
    }
    let count = preview
        .files
        .iter()
        .filter(|s| **s == State::Verified)
        .count();
    if count == 0 {
        return Err(invalid("没有通过校验的剩余文件"));
    }
    let (stage, destination) = staging(destination, selected)?;
    for (entry, state) in preview.manifest.files.iter().zip(&preview.files) {
        if *state != State::Verified {
            continue;
        }
        let target = stage.0.join(&entry.path);
        fs::create_dir_all(target.parent().ok_or_else(|| invalid("无效恢复路径"))?)?;
        let source = safe_path(selected, &Path::new("files").join(&entry.path))?;
        if transfer(&source, Some(&target))? != (entry.bytes, entry.sha256.clone()) {
            return Err(invalid("恢复期间文件发生变化，未提交副本"));
        }
    }
    if inspect_unlocked(selected)? != preview {
        return Err(invalid("恢复期间中断记录发生变化，未提交副本"));
    }
    move_no_replace(&stage.0, &destination)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf, PathBuf, PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("inkstone-partial-backup-{}", unique_id()));
            fs::create_dir_all(root.join("vault")).unwrap();
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            for (name, body) in [
                ("keep.md", "可恢复 👩‍💻"),
                ("missing.md", "missing"),
                ("corrupt.md", "before"),
            ] {
                fs::write(vault.root.join(name), body).unwrap();
            }
            let directory = root.join("backups");
            fs::create_dir(&directory).unwrap();
            let selected = directory.join(".inkstone-backup-cleanup-fixture");
            create(&vault, &selected).unwrap();
            Self(root, directory, selected)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn partial_restore_copies_only_verified_files_and_keeps_all_residuals() {
        let f = Fixture::new();
        fs::remove_file(f.2.join("files/missing.md")).unwrap();
        fs::write(f.2.join("files/corrupt.md"), "broken").unwrap();
        fs::write(f.2.join("files/unlisted.md"), "preserve extra").unwrap();
        let manifest = fs::read(f.2.join("manifest.json")).unwrap();
        let preview = inspect(&f.1, &f.2).unwrap();
        let states: Vec<_> = preview
            .manifest
            .files
            .iter()
            .zip(&preview.files)
            .map(|(e, s)| (e.path.to_string_lossy().into_owned(), s.clone()))
            .collect();
        assert_eq!(
            states,
            vec![
                (
                    "corrupt.md".into(),
                    State::Unavailable("内容校验失败".into())
                ),
                ("keep.md".into(), State::Verified),
                ("missing.md".into(), State::Missing)
            ]
        );
        let destination = f.0.join("copy");
        assert_eq!(
            restore_verified(&f.1, &f.2, &destination, &preview).unwrap(),
            1
        );
        assert_eq!(
            fs::read_to_string(destination.join("keep.md")).unwrap(),
            "可恢复 👩‍💻"
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
        assert_eq!(fs::read(f.2.join("manifest.json")).unwrap(), manifest);
        assert_eq!(
            fs::read_to_string(f.2.join("files/corrupt.md")).unwrap(),
            "broken"
        );
        assert_eq!(
            fs::read_to_string(f.2.join("files/unlisted.md")).unwrap(),
            "preserve extra"
        );
        assert_eq!(capacity::list(&f.1).unwrap().interrupted, vec![f.2.clone()]);
        assert!(cleanup::retain_interrupted(&f.1, &f.2).is_err());
    }

    #[test]
    fn stale_preview_empty_recovery_and_existing_destination_never_publish() {
        let f = Fixture::new();
        let preview = inspect(&f.1, &f.2).unwrap();
        let destination = f.0.join("copy");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), "original").unwrap();
        assert!(restore_verified(&f.1, &f.2, &destination, &preview).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("sentinel")).unwrap(),
            "original"
        );
        fs::remove_file(f.2.join("files/keep.md")).unwrap();
        let fresh = f.0.join("fresh");
        assert!(restore_verified(&f.1, &f.2, &fresh, &preview).is_err());
        assert!(!fresh.exists());
        fs::remove_dir_all(f.2.join("files")).unwrap();
        let empty = inspect(&f.1, &f.2).unwrap();
        assert!(empty.files.iter().all(|state| *state == State::Missing));
        assert!(restore_verified(&f.1, &f.2, &fresh, &empty).is_err());
        assert!(!fresh.exists());
        assert_eq!(
            fs::read_dir(&f.0).unwrap().count(),
            4,
            "no staging leftovers"
        );
    }

    #[test]
    fn invalid_selection_or_manifest_cannot_escape_the_record() {
        let f = Fixture::new();
        assert!(inspect(&f.0, &f.2).is_err());
        let path = f.2.join("manifest.json");
        let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.files[0].path = PathBuf::from("../../outside");
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(inspect(&f.1, &f.2).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_payload_is_unavailable_and_never_copied() {
        let f = Fixture::new();
        fs::remove_dir_all(f.2.join("files")).unwrap();
        std::os::unix::fs::symlink(f.0.join("vault"), f.2.join("files")).unwrap();
        let preview = inspect(&f.1, &f.2).unwrap();
        assert!(
            preview
                .files
                .iter()
                .all(|state| matches!(state, State::Unavailable(_)))
        );
        assert!(restore_verified(&f.1, &f.2, &f.0.join("copy"), &preview).is_err());
        assert_eq!(
            fs::read_to_string(f.0.join("vault/keep.md")).unwrap(),
            "可恢复 👩‍💻"
        );
    }
}
