//! Durable journal ownership after application-driven note renames.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
mod checkpoint;
mod intent;

#[derive(Serialize, Deserialize)]
struct Links {
    version: u32,
    root: PathBuf,
    owners: BTreeMap<PathBuf, PathBuf>,
}
pub(super) struct Ownership(Links);
fn path(vault: &Vault) -> PathBuf {
    let key = serde_json::to_vec(&vault.root).expect("path serializes");
    vault
        .recovery_dir
        .join(format!("{:x}.history-links", Sha256::digest(key)))
}
fn invalid() -> VaultError {
    io::Error::new(io::ErrorKind::InvalidData, "invalid history ownership map").into()
}
impl Ownership {
    pub(super) fn load(vault: &Vault) -> Result<Self, VaultError> {
        intent::reconcile(vault)?;
        let path = path(vault);
        let committed = checkpoint::read(vault)?;
        let has_commit = committed.is_some();
        let links = if let Some(links) = committed {
            links
        } else {
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Links {
                    version: 1,
                    root: vault.root.clone(),
                    owners: BTreeMap::new(),
                },
                Err(error) => return Err(error.into()),
                Ok(meta) => {
                    if !meta.is_file() || is_reparse(&meta) {
                        return Err(invalid());
                    }
                    serde_json::from_slice::<Links>(&fs::read(path)?).map_err(|_| invalid())?
                }
            }
        };
        if links.version != 1 || links.root != vault.root {
            return Err(invalid());
        }
        for (journal, relative) in &links.owners {
            if journal.parent() != Some(vault.recovery_dir.as_path())
                || journal.extension().is_some()
            {
                return Err(invalid());
            }
            Vault::validate_relative(relative)?;
        }
        if has_commit {
            checkpoint::repair_cache(vault, &links);
        }
        Ok(Self(links))
    }
    pub(super) fn owns(&self, journal: &Path, original: &Path, requested: &Path) -> bool {
        self.owner(journal, original) == requested
    }

    pub(super) fn owner<'a>(&'a self, journal: &Path, original: &'a Path) -> &'a Path {
        self.0
            .owners
            .get(&journal.with_extension(""))
            .map(PathBuf::as_path)
            .unwrap_or(original)
    }
}

fn lock(vault: &Vault) -> Result<fs::File, VaultError> {
    let lock_path = path(vault).with_extension("history-lock");
    if let Ok(meta) = fs::symlink_metadata(&lock_path)
        && (!meta.is_file() || is_reparse(&meta))
    {
        return Err(invalid());
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock().map_err(io::Error::other)?;
    Ok(lock)
}

/// Stage metadata before moving. Publication failure rolls the note back.
/// Interrupted publication is reconciled using the persisted filesystem identity.
pub(in crate::vault) fn rename(
    vault: &Vault,
    old: &Path,
    new: &Path,
    source: &Path,
    dest: &Path,
) -> Result<(), VaultError> {
    rename_with(vault, old, new, source, dest, |from, to| {
        fs::rename(from, to)
    })
}
fn rename_with(
    vault: &Vault,
    old: &Path,
    new: &Path,
    source: &Path,
    dest: &Path,
    publish: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), VaultError> {
    let _lock = lock(vault)?;
    intent::reconcile_locked(vault)?;
    let mut ownership = Ownership::load(vault)?;
    let before = checkpoint::encode(&ownership.0)?;
    if source.is_dir() {
        // Scan journals once, using current ownership so earlier note renames
        // follow their containing folder. Deleted notes do not move with it.
        for item in fs::read_dir(&vault.recovery_dir)? {
            let journal = item?.path();
            if !journal
                .extension()
                .is_some_and(|e| e == "json" || e == "saved")
            {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&journal) else {
                continue;
            };
            let Some(scope) = super::records::scope(vault, &journal, &meta) else {
                continue;
            };
            if scope.root != vault.root {
                continue;
            }
            let identity = journal.with_extension("");
            let current = ownership.0.owners.get(&identity).unwrap_or(&scope.relative);
            let Ok(suffix) = current.strip_prefix(old) else {
                continue;
            };
            if !vault.path(current).is_ok_and(|p| p.is_file()) {
                continue;
            }
            let destination = new.join(suffix);
            Vault::validate_relative(&destination)?;
            ownership.0.owners.insert(identity, destination);
        }
    } else {
        for entry in vault.history(old)? {
            ownership
                .0
                .owners
                .insert(entry.journal.with_extension(""), new.to_owned());
        }
    }
    let target = checkpoint::path(vault);
    let staged = target.with_extension(format!("{}.pending-links", unique_id()));
    let bytes = checkpoint::encode(&ownership.0)?;
    if let Err(error) = write_new_synced(&staged, &bytes) {
        let _ = fs::remove_file(&staged);
        return Err(error.into());
    }
    let pending = match intent::prepare(vault, old, new, source, before, bytes, staged.clone()) {
        Ok(pending) => pending,
        Err(error) => {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
    };
    let previous_proof = match checkpoint::invalidate_proof(vault) {
        Ok(previous) => previous,
        Err(error) => {
            pending.clear(vault);
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
    };
    if let Err(error) = move_no_replace(source, dest) {
        checkpoint::restore_proof(vault, previous_proof);
        pending.clear(vault);
        let _ = fs::remove_file(&staged);
        return Err(error.into());
    }
    if let Err(error) = publish(&staged, &target) {
        let rollback = move_no_replace(dest, source);
        let _ = fs::remove_file(&staged);
        return match rollback {
            Ok(()) => {
                checkpoint::restore_proof(vault, previous_proof);
                pending.clear(vault);
                Err(error.into())
            },
            Err(rollback) => Err(io::Error::other(format!("history mapping failed: {error}; note remains at {} because rollback failed: {rollback}", dest.display())).into()),
        };
    }
    checkpoint::repair_cache(vault, &ownership.0);
    checkpoint::certify(vault, &ownership.0);
    pending.clear(vault);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renames_transfer_exact_journals_across_restart_and_path_reuse() {
        let root = std::env::temp_dir().join(format!("inkstone-history-links-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let a = Path::new("a.md");
        let b = Path::new("b.md");
        let c = Path::new("c.md");
        let first = vault.save(a, None, "first").unwrap();
        vault.save(a, Some("first"), "second").unwrap();
        vault.rename_note(a, b, "second").unwrap();
        let vault = Vault::open(&vault.root, &vault.recovery_dir).unwrap();
        assert_eq!(vault.history(b).unwrap().len(), 3);
        assert!(vault.history(a).unwrap().is_empty());
        assert_eq!(
            vault.read_history(b, &first.recovery).unwrap().draft,
            "first"
        );
        assert!(vault.read_history(a, &first.recovery).is_err());
        vault.save(a, None, "different note").unwrap();
        assert_eq!(vault.history(a).unwrap().len(), 1);
        assert_eq!(vault.history(b).unwrap().len(), 3);
        vault.rename_note(b, c, "second").unwrap();
        assert_eq!(vault.history(c).unwrap().len(), 4);
        assert!(vault.history(b).unwrap().is_empty());
        vault.save(b, None, "another note").unwrap();
        assert_eq!(vault.history(b).unwrap().len(), 1);
        assert_eq!(vault.read_history(c, &first.recovery).unwrap().relative, a);
        assert!(vault.rename_note(c, a, "second").is_err());
        assert_eq!(vault.read(c).unwrap().as_deref(), Some("second"));
        assert_eq!(vault.read(a).unwrap().as_deref(), Some("different note"));
        assert!(vault.read_history(c, &first.recovery).is_ok());
        // Bad ownership metadata fails closed before the file can move.
        fs::write(checkpoint::path(&vault), b"broken").unwrap();
        fs::write(path(&vault), b"broken mirror").unwrap();
        assert!(vault.rename_note(c, Path::new("d.md"), "second").is_err());
        assert!(vault.read(c).unwrap().is_some());
        assert!(vault.read(Path::new("d.md")).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn publication_failure_rolls_back_without_overwriting_a_concurrent_file() {
        let root = std::env::temp_dir().join(format!("inkstone-links-rollback-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = Path::new("old.md");
        let new = Path::new("new.md");
        let record = vault.save(old, None, "original").unwrap();
        let source = vault.path(old).unwrap();
        let dest = vault.path(new).unwrap();
        let fail = |_: &Path, _: &Path| Err(io::Error::other("disk full"));
        assert!(rename_with(&vault, old, new, &source, &dest, fail).is_err());
        assert_eq!(vault.read(old).unwrap().as_deref(), Some("original"));
        assert!(vault.read(new).unwrap().is_none());
        assert!(vault.read_history(old, &record.recovery).is_ok());
        assert!(vault.history(new).unwrap().is_empty());
        let error = rename_with(&vault, old, new, &source, &dest, |_, _| {
            fs::write(&source, "external").unwrap();
            Err(io::Error::other("disk full"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("rollback failed"));
        assert_eq!(vault.read(old).unwrap().as_deref(), Some("external"));
        assert_eq!(vault.read(new).unwrap().as_deref(), Some("original"));
        assert!(record.recovery.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn folder_rename_transfers_nested_and_previously_renamed_notes_only() {
        let root = std::env::temp_dir().join(format!("inkstone-folder-history-{}", unique_id()));
        fs::create_dir_all(root.join("vault/old/nested")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let original = Path::new("old/nested/a.md");
        let renamed = Path::new("old/nested/b.md");
        let first = vault.save(original, None, "first").unwrap();
        vault.rename_note(original, renamed, "first").unwrap();
        let top = vault.save(Path::new("old/top.md"), None, "top").unwrap();
        let deleted = vault
            .save(Path::new("old/deleted.md"), None, "deleted")
            .unwrap();
        fs::remove_file(vault.path(Path::new("old/deleted.md")).unwrap()).unwrap();
        fs::create_dir_all(vault.root.join("old-other")).unwrap();
        let unrelated = vault
            .save(Path::new("old-other/a.md"), None, "other")
            .unwrap();
        fs::write(vault.root.join("old/nested/asset.bin"), [0, 1, 255]).unwrap();
        assert!(
            rename_with(
                &vault,
                Path::new("old"),
                Path::new("new"),
                &vault.root.join("old"),
                &vault.root.join("new"),
                |_, _| Err(io::Error::other("mapping publication failed")),
            )
            .is_err()
        );
        assert!(vault.root.join("old/nested/asset.bin").is_file());
        assert!(!vault.root.join("new").exists());
        assert!(vault.read_history(renamed, &first.recovery).is_ok());
        vault
            .rename_folder(Path::new("old"), Path::new("new"))
            .unwrap();
        let vault = Vault::open(&vault.root, &vault.recovery_dir).unwrap();
        assert_eq!(
            vault.history(Path::new("new/nested/b.md")).unwrap().len(),
            2
        );
        assert!(
            vault
                .read_history(Path::new("new/nested/b.md"), &first.recovery)
                .is_ok()
        );
        assert!(
            vault
                .read_history(Path::new("new/top.md"), &top.recovery)
                .is_ok()
        );
        assert!(vault.history(renamed).unwrap().is_empty());
        assert!(
            vault
                .read_history(Path::new("old/deleted.md"), &deleted.recovery)
                .is_ok()
        );
        assert!(
            vault
                .read_history(Path::new("old-other/a.md"), &unrelated.recovery)
                .is_ok()
        );
        assert_eq!(
            fs::read(vault.root.join("new/nested/asset.bin")).unwrap(),
            [0, 1, 255]
        );
        fs::create_dir_all(vault.root.join("old/nested")).unwrap();
        vault.save(renamed, None, "new identity").unwrap();
        assert_eq!(vault.history(renamed).unwrap().len(), 1);
        vault
            .rename_folder(Path::new("new"), Path::new("again"))
            .unwrap();
        assert!(
            vault
                .read_history(Path::new("again/nested/b.md"), &first.recovery)
                .is_ok()
        );
        assert!(
            vault
                .rename_folder(Path::new("again"), Path::new("old"))
                .is_err()
        );
        assert!(
            vault
                .read_history(Path::new("again/nested/b.md"), &first.recovery)
                .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
