//! Durable rename intent, resolved by filesystem identity rather than path presence.
use super::*;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: u64,
    created: Option<SystemTime>,
}
fn identity(path: &Path) -> Result<Identity, VaultError> {
    let meta = fs::symlink_metadata(path)?;
    if is_reparse(&meta) || (!meta.is_file() && !meta.is_dir()) {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Identity {
            volume: meta.dev(),
            file: meta.ino(),
            created: meta.created().ok(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let (volume, high, low) = SaveGuard::identity(&file)?;
        Ok(Identity {
            volume: volume.into(),
            file: (u64::from(high) << 32) | u64::from(low),
            created: meta.created().ok(),
        })
    }
}
#[derive(Serialize, Deserialize)]
pub(super) struct Intent {
    root: PathBuf,
    old: PathBuf,
    new: PathBuf,
    identity: Identity,
    before: Vec<u8>,
    after: Vec<u8>,
    proof: Option<Vec<u8>>,
    staged: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    intent: Intent,
    checksum: String,
}
fn path(vault: &Vault) -> PathBuf {
    super::path(vault).with_extension("history-intent")
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Intent {
    pub(super) fn clear(&self, vault: &Vault) {
        let _ = fs::remove_file(&self.staged);
        let _ = fs::remove_file(path(vault));
    }
}
pub(super) fn prepare(
    vault: &Vault,
    old: &Path,
    new: &Path,
    source: &Path,
    before: Vec<u8>,
    after: Vec<u8>,
    staged: PathBuf,
) -> Result<Intent, VaultError> {
    let intent = Intent {
        root: vault.root.clone(),
        old: old.to_owned(),
        new: new.to_owned(),
        identity: identity(source)?,
        before,
        after,
        staged,
        proof: match fs::read(checkpoint::proof_path(vault)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        },
    };
    #[derive(Serialize)]
    struct Borrowed<'a> {
        intent: &'a Intent,
        checksum: String,
    }
    let bytes = serde_json::to_vec(&intent).map_err(io::Error::other)?;
    let encoded = serde_json::to_vec(&Borrowed {
        intent: &intent,
        checksum: digest(&bytes),
    })
    .map_err(io::Error::other)?;
    let temp = file_temp(vault);
    let result =
        write_new_synced(&temp, &encoded).and_then(|()| move_no_replace(&temp, &path(vault)));
    let _ = fs::remove_file(temp);
    result?;
    Ok(intent)
}
pub(super) fn reconcile(vault: &Vault) -> Result<(), VaultError> {
    match fs::symlink_metadata(path(vault)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let _lock = super::lock(vault)?;
    reconcile_locked(vault)
}
pub(super) fn reconcile_locked(vault: &Vault) -> Result<(), VaultError> {
    let file = path(vault);
    let meta = match fs::symlink_metadata(&file) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || is_reparse(&meta) {
        return Err(invalid());
    }
    let envelope: Envelope = serde_json::from_slice(&fs::read(file)?).map_err(|_| invalid())?;
    let intent = envelope.intent;
    if digest(&serde_json::to_vec(&intent).map_err(io::Error::other)?) != envelope.checksum
        || intent.root != vault.root
    {
        return Err(invalid());
    }
    if intent.staged.parent() != Some(vault.recovery_dir.as_path())
        || intent
            .staged
            .extension()
            .is_none_or(|e| e != "pending-links")
    {
        return Err(invalid());
    }
    // regular_file_path also accepts directory paths, validating confinement and links.
    let source = vault.regular_file_path(&intent.old)?;
    let dest = vault.regular_file_path(&intent.new)?;
    let current = checkpoint::read_primary(vault)?
        .map(|links| checkpoint::encode(&links))
        .transpose()?;
    if current.as_deref() == Some(intent.after.as_slice()) {
        if let Some(links) = checkpoint::read_primary(vault)? {
            checkpoint::repair_cache(vault, &links);
            checkpoint::certify(vault, &links);
        }
        intent.clear(vault);
        return Ok(());
    }
    // The checkpoint may be absent before the first rename. Compare the legacy map.
    let before = match current {
        Some(bytes) => bytes,
        None => {
            let links = match fs::read(super::path(vault)) {
                Ok(bytes) => serde_json::from_slice::<Links>(&bytes).map_err(|_| invalid())?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => Links {
                    version: 1,
                    root: vault.root.clone(),
                    owners: BTreeMap::new(),
                },
                Err(e) => return Err(e.into()),
            };
            checkpoint::encode(&links)?
        }
    };
    if before != intent.before {
        return Err(invalid());
    }
    let source_matches = identity(&source).is_ok_and(|id| id == intent.identity);
    let dest_matches = identity(&dest).is_ok_and(|id| id == intent.identity);
    if source_matches && !dest_matches {
        checkpoint::restore_proof(vault, intent.proof.clone());
        intent.clear(vault);
        return Ok(());
    }
    if !source_matches && dest_matches {
        let staged = file_temp(vault);
        write_new_synced(&staged, &intent.after)?;
        let result = fs::rename(&staged, checkpoint::path(vault));
        let _ = fs::remove_file(staged);
        result?;
        if let Some(links) = checkpoint::read(vault)? {
            checkpoint::repair_cache(vault, &links);
            checkpoint::certify(vault, &links);
        }
        intent.clear(vault);
        return Ok(());
    }
    Err(
        io::Error::other("cannot determine interrupted rename outcome; history intent retained")
            .into(),
    )
}
fn file_temp(vault: &Vault) -> PathBuf {
    path(vault).with_extension(format!("{}.reconcile-temp", unique_id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_move_and_publication_reconcile_files_and_folders() {
        for folder in [false, true] {
            for published in [false, true] {
                let root = std::env::temp_dir().join(format!("inkstone-intent-{}", unique_id()));
                fs::create_dir_all(root.join("vault/old")).unwrap();
                let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
                let note = Path::new("old/note.md");
                let record = vault.save(note, None, "original").unwrap();
                let old = if folder { Path::new("old") } else { note };
                let new = if folder {
                    Path::new("new")
                } else {
                    Path::new("new.md")
                };
                let destination = if folder {
                    Path::new("new/note.md")
                } else {
                    new
                };
                let stopped = std::panic::catch_unwind(|| {
                    super::super::rename_with(
                        &vault,
                        old,
                        new,
                        &vault.root.join(old),
                        &vault.root.join(new),
                        |from, to| {
                            if published {
                                fs::rename(from, to).unwrap();
                            }
                            panic!("simulated process exit");
                        },
                    )
                    .unwrap();
                });
                assert!(stopped.is_err());
                assert!(path(&vault).is_file());
                let reopened = Vault::open(&vault.root, &vault.recovery_dir).unwrap();
                assert!(reopened.read_history(destination, &record.recovery).is_ok());
                assert!(!path(&vault).exists());
                assert!(reopened.history(note).unwrap().is_empty());
                assert_eq!(
                    reopened.read(destination).unwrap().as_deref(),
                    Some("original")
                );
                fs::remove_dir_all(root).unwrap();
            }
        }
    }
    #[test]
    fn unmoved_intent_aborts_but_replaced_identity_remains_unresolved() {
        let root = std::env::temp_dir().join(format!("inkstone-intent-before-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = Path::new("old.md");
        let new = Path::new("new.md");
        let record = vault.save(old, None, "body").unwrap();
        for replaced in [false, true] {
            let before = Ownership::load(&vault).unwrap();
            let encoded = checkpoint::encode(&before.0).unwrap();
            prepare(
                &vault,
                old,
                new,
                &vault.root.join(old),
                encoded.clone(),
                encoded,
                vault.recovery_dir.join("test.pending-links"),
            )
            .unwrap();
            checkpoint::invalidate_proof(&vault).unwrap();
            if replaced {
                fs::rename(vault.root.join(old), vault.root.join("held.md")).unwrap();
                fs::write(vault.root.join(new), "body").unwrap();
                assert!(vault.history(new).is_err());
                assert!(path(&vault).exists());
                assert_eq!(vault.read(new).unwrap().as_deref(), Some("body"));
            } else {
                assert!(vault.read_history(old, &record.recovery).is_ok());
                assert!(!path(&vault).exists());
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn subprocess_exit_after_move_preserves_history_on_reopen() {
        const CHILD: &str = "INKSTONE_HISTORY_INTENT_CHILD";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            super::super::rename_with(
                &vault,
                Path::new("old.md"),
                Path::new("new.md"),
                &vault.root.join("old.md"),
                &vault.root.join("new.md"),
                |_, _| {
                    std::process::exit(73);
                },
            )
            .unwrap();
            unreachable!();
        }
        let root = std::env::temp_dir().join(format!("inkstone-intent-process-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let record = vault
            .save(Path::new("old.md"), None, "durable body")
            .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "vault::history::links::intent::tests::subprocess_exit_after_move_preserves_history_on_reopen"])
            .env(CHILD, &root).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(path(&vault).is_file());
        let reopened = Vault::open(&vault.root, &vault.recovery_dir).unwrap();
        assert_eq!(
            reopened
                .read_history(Path::new("new.md"), &record.recovery)
                .unwrap()
                .draft,
            "durable body"
        );
        assert!(!path(&vault).exists());
        assert!(!fs::read_dir(&vault.recovery_dir).unwrap().any(|e| {
            e.unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "pending-links")
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
