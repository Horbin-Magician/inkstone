//! One document's background draft lifetime, separate from Markdown saves.
//!
//! Keep this owner across writes (and serialize access from background tasks).
//! Dropping it deliberately retains the last persisted draft for crash recovery.
use super::*;

impl Vault {
    /// Metadata only; warm entries never deserialize journal bodies.
    /// Corrupt journals remain on disk for manual inspection.
    pub fn recovery_summaries(&self) -> Result<Vec<RecoverySummary>, VaultError> {
        let mut entries = vec![];
        for entry in fs::read_dir(&self.recovery_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !is_reparse(&metadata) => metadata,
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if let Some(scope) = history::records::scope(self, &path, &metadata)
                && scope.root == self.root
            {
                entries.push(RecoverySummary {
                    journal: path,
                    relative: scope.relative,
                    modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                    bytes: metadata.len(),
                });
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
        Ok(entries)
    }

    /// Load precisely this pending record, without falling back to saved history.
    pub fn read_draft(&self, summary: &RecoverySummary) -> Result<RecoveryEntry, VaultError> {
        Self::validate_relative(&summary.relative)?;
        if summary.journal.parent() != Some(self.recovery_dir.as_path())
            || summary.journal.extension().is_none_or(|e| e != "json")
        {
            return Err(VaultError::InvalidPath);
        }
        let unchanged = || -> Result<(), VaultError> {
            let metadata = fs::symlink_metadata(&summary.journal)?;
            if !metadata.is_file()
                || is_reparse(&metadata)
                || metadata.len() != summary.bytes
                || metadata.modified().unwrap_or(UNIX_EPOCH) != summary.modified
            {
                return Err(VaultError::InvalidPath);
            }
            Ok(())
        };
        unchanged()?;
        let record: Recovery =
            serde_json::from_reader(io::BufReader::new(fs::File::open(&summary.journal)?))
                .map_err(io::Error::other)?;
        unchanged()?;
        if record.root != self.root || record.relative != summary.relative {
            return Err(VaultError::InvalidPath);
        }
        Ok(RecoveryEntry {
            journal: summary.journal.clone(),
            record,
            modified: summary.modified,
            bytes: summary.bytes,
        })
    }

    /// Verify the exact reviewed crash record, never a completed-save fallback.
    pub fn validate_draft(&self, entry: &RecoveryEntry) -> Result<(), VaultError> {
        if entry.journal.parent() != Some(self.recovery_dir.as_path())
            || entry.journal.extension().is_none_or(|e| e != "json")
        {
            return Err(VaultError::InvalidPath);
        }
        let metadata = fs::symlink_metadata(&entry.journal)?;
        if !metadata.is_file() || is_reparse(&metadata) {
            return Err(VaultError::InvalidPath);
        }
        if self.read_history(&entry.record.relative, &entry.journal)? != entry.record {
            return Err(VaultError::InvalidPath);
        }
        Ok(())
    }

    /// Remove only the reviewed record; concurrent new snapshots remain recoverable.
    pub fn discard_draft(&self, entry: &RecoveryEntry) -> Result<(), VaultError> {
        self.validate_draft(entry)?;
        fs::remove_file(&entry.journal)?;
        history::records::forget(self, &entry.journal);
        Ok(())
    }
}

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
            Ok(()) => {
                history::records::forget(&self.vault, path);
                false
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                history::records::forget(&self.vault, path);
                false
            }
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
    fn summaries_load_only_selected_pending_records_and_reject_stale_selections() {
        let f = Fixture::new();
        let mut session = f.session();
        let text = "草稿😀\n".repeat(100_000);
        session.persist(Some("original"), &text).unwrap();
        let other_root = f.0.join("other");
        fs::create_dir(&other_root).unwrap();
        let other = Vault::open(other_root, f.1.recovery_dir.clone()).unwrap();
        other
            .journal(Path::new("foreign.md"), None, "foreign body")
            .unwrap();
        fs::write(f.1.recovery_dir.join("corrupt.json"), b"broken").unwrap();
        let summaries = f.1.recovery_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        let selected = &summaries[0];
        assert_eq!(selected.relative, Path::new("note.md"));
        assert!(selected.bytes > 1_000_000);
        let reviewed = f.1.read_draft(selected).unwrap();
        assert_eq!(reviewed.record.draft, text);
        assert_eq!(reviewed.record.baseline.as_deref(), Some("original"));
        assert_eq!(reviewed.bytes, selected.bytes);
        assert!(other.read_draft(selected).is_err());
        let mut wrong_path = selected.clone();
        wrong_path.relative = "another.md".into();
        assert!(f.1.read_draft(&wrong_path).is_err());
        // Listing is not permission to act on a changed or published record.
        fs::write(&selected.journal, b"changed").unwrap();
        assert!(f.1.read_draft(selected).is_err());
        fs::write(
            &selected.journal,
            serde_json::to_vec(&reviewed.record).unwrap(),
        )
        .unwrap();
        let refreshed = f.1.recovery_summaries().unwrap().remove(0);
        let saved = refreshed.journal.with_extension("saved");
        fs::rename(&refreshed.journal, &saved).unwrap();
        assert!(f.1.read_draft(&refreshed).is_err());
        assert!(f.1.recovery_summaries().unwrap().is_empty());
        assert!(saved.exists());
        assert_eq!(
            fs::read_to_string(f.1.root.join("note.md")).unwrap(),
            "original"
        );
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
        let metadata = fs::metadata(&entries[0].journal).unwrap();
        assert_eq!(entries[0].bytes, metadata.len());
        assert_eq!(entries[0].modified, metadata.modified().unwrap());
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
        assert_eq!(f.1.recoveries().unwrap().len(), 1);
        assert!(DraftSession::new(f.1.clone(), "../escape.md".into()).is_err());
    }
    #[test]
    fn discard_checks_reviewed_record_and_keeps_other_versions() {
        let f = Fixture::new();
        f.1.journal(Path::new("note.md"), Some("original"), "older")
            .unwrap();
        f.1.journal(Path::new("note.md"), Some("original"), "newer")
            .unwrap();
        let entries = f.1.recoveries().unwrap();
        assert_eq!(entries.len(), 2);
        let reviewed = entries.iter().find(|e| e.record.draft == "newer").unwrap();
        let mut stale = reviewed.clone();
        stale.record.draft = "unreviewed".into();
        assert!(f.1.discard_draft(&stale).is_err());
        assert!(reviewed.journal.exists());
        let mut outside = reviewed.clone();
        outside.journal = f.1.root.join("note.md");
        assert!(f.1.discard_draft(&outside).is_err());
        f.1.discard_draft(reviewed).unwrap();
        assert_eq!(f.1.recoveries().unwrap()[0].record.draft, "older");
        assert_eq!(
            fs::read_to_string(f.1.root.join("note.md")).unwrap(),
            "original"
        );
    }
}
