//! One document's background draft lifetime, separate from Markdown saves.
//!
//! Keep this owner across writes (and serialize access from background tasks).
//! Dropping it deliberately retains the last persisted draft for crash recovery.
use super::*;

pub struct DraftSession {
    vault: Vault,
    relative: PathBuf,
    current: Option<PathBuf>,
    obsolete: Vec<PathBuf>,
}

impl DraftSession {
    pub fn new(vault: Vault, relative: PathBuf) -> Result<Self, VaultError> {
        Vault::validate_relative(&relative)?;
        Ok(Self {
            vault,
            relative,
            current: None,
            obsolete: Vec::new(),
        })
    }

    /// Publish a complete, synced journal before retiring the previous draft.
    /// A failed write leaves the previous recovery available; callers must retry.
    /// This never reads or writes the authoritative Markdown file.
    pub fn persist(&mut self, baseline: Option<&str>, text: &str) -> Result<(), VaultError> {
        let record = Recovery {
            root: self.vault.root.clone(),
            relative: self.relative.clone(),
            baseline: baseline.map(str::to_owned),
            draft: text.to_owned(),
        };
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        let pending = self
            .vault
            .recovery_dir
            .join(format!("{}.pending", unique_id()));
        let published = pending.with_extension("json");
        let result = (|| -> io::Result<()> {
            write_new_synced(&pending, &bytes)?;
            fs::rename(&pending, &published)?;
            #[cfg(unix)]
            fs::File::open(&self.vault.recovery_dir)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&pending);
            // A directory sync may fail after publication. Keep the valid copy
            // tracked so a later successful retry or clear can retire it.
            if published.exists() {
                self.obsolete.push(published);
            }
            return Err(error.into());
        }
        if let Some(previous) = self.current.replace(published) {
            self.obsolete.push(previous);
        }
        self.remove_obsolete()
    }

    /// Call only after saving this snapshot, or explicitly discarding it.
    /// Serialize this with persist so an in-flight write cannot resurrect it.
    pub fn clear(&mut self) -> Result<(), VaultError> {
        if let Some(current) = self.current.take() {
            self.obsolete.push(current);
        }
        self.remove_obsolete()
    }

    fn remove_obsolete(&mut self) -> Result<(), VaultError> {
        let mut error = None;
        self.obsolete.retain(|path| match fs::remove_file(path) {
            Ok(()) => false,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => {
                error = Some(e);
                true
            }
        });
        error.map_or(Ok(()), |error| Err(error.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf, Vault);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("inkstone-drafts-{}", unique_id()));
            fs::create_dir_all(root.join("vault")).unwrap();
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            fs::write(vault.root.join("note.md"), "original").unwrap();
            Self(root, vault)
        }
        fn session(&self) -> DraftSession {
            DraftSession::new(self.1.clone(), "note.md".into()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn latest_draft_survives_restart_without_modifying_markdown() {
        let f = Fixture::new();
        let mut session = f.session();
        for text in ["first", "中文😀\r\nlatest"] {
            session.persist(Some("original"), text).unwrap();
        }
        drop(session);
        let reopened = Vault::open(&f.1.root, &f.1.recovery_dir).unwrap();
        let entries = reopened.recoveries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].record.draft, "中文😀\r\nlatest");
        assert_eq!(entries[0].record.baseline.as_deref(), Some("original"));
        assert_eq!(fs::read_dir(&f.1.recovery_dir).unwrap().count(), 1);
        assert_eq!(
            fs::read_to_string(f.1.root.join("note.md")).unwrap(),
            "original"
        );
    }

    #[test]
    fn failed_write_retains_previous_draft_and_can_retry() {
        let f = Fixture::new();
        let mut session = f.session();
        session.persist(Some("original"), "first").unwrap();
        let held = f.0.join("held");
        fs::rename(&f.1.recovery_dir, &held).unwrap();
        fs::write(&f.1.recovery_dir, "not a directory").unwrap();
        assert!(session.persist(Some("original"), "second").is_err());
        fs::remove_file(&f.1.recovery_dir).unwrap();
        fs::rename(held, &f.1.recovery_dir).unwrap();
        assert_eq!(f.1.recoveries().unwrap()[0].record.draft, "first");
        session.persist(Some("original"), "second").unwrap();
        assert_eq!(f.1.recoveries().unwrap()[0].record.draft, "second");
        assert_eq!(fs::read_dir(&f.1.recovery_dir).unwrap().count(), 1);
    }

    #[test]
    fn clear_only_removes_owned_drafts_and_is_idempotent() {
        let f = Fixture::new();
        let unrelated =
            f.1.journal(Path::new("note.md"), Some("old"), "conflict")
                .unwrap();
        let mut session = f.session();
        session.persist(Some("original"), "new").unwrap();
        session.clear().unwrap();
        session.clear().unwrap();
        assert!(unrelated.exists());
        assert_eq!(fs::read_dir(&f.1.recovery_dir).unwrap().count(), 1);
        assert!(DraftSession::new(f.1.clone(), "../escape.md".into()).is_err());
    }
}
