use super::*;
mod index;
mod records;

pub(super) fn cache_journal(vault: &Vault, path: &Path, recovery: &Recovery) {
    records::seed(vault, path, recovery);
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    /// Zero disables this limit. Failed drafts and the latest version are retained.
    pub days: u64,
    pub max_mib: u64,
}
impl Default for Retention {
    fn default() -> Self {
        Self {
            days: 30,
            max_mib: 128,
        }
    }
}

/// Metadata only: selecting an entry loads its body on a background thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub journal: PathBuf,
    pub modified: SystemTime,
    pub bytes: u64,
    pub saved: bool,
}

impl Vault {
    /// Archive local edits and reject disk changes made since the review opened.
    pub fn resolve_conflict(
        &self,
        relative: &Path,
        baseline: Option<&str>,
        draft: &str,
        expected_disk: Option<&str>,
        replacement: Option<&str>,
    ) -> Result<String, VaultError> {
        let recovery = self.journal(relative, baseline, draft)?;
        if self.read(relative)?.as_deref() != expected_disk {
            return Err(VaultError::Conflict { recovery });
        }
        if let Some(text) = replacement {
            Ok(self.save(relative, expected_disk, text)?.text)
        } else {
            expected_disk
                .map(str::to_string)
                .ok_or(VaultError::InvalidPath)
        }
    }

    pub fn history(&self, relative: &Path) -> Result<Vec<HistoryEntry>, VaultError> {
        Self::validate_relative(relative)?;
        if let Some(entries) = index::load(self, relative) {
            return Ok(entries);
        }
        let directory_modified = index::stamp(self);
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.recovery_dir)? {
            let path = item?.path();
            let saved = path.extension().is_some_and(|e| e == "saved");
            if !saved && path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) if m.is_file() && !is_reparse(&m) => m,
                _ => continue,
            };
            if records::scope(self, &path, &metadata)
                .is_none_or(|scope| scope.root != self.root || scope.relative != relative)
            {
                // Partial journals and unrelated vaults are never modified.
                continue;
            }
            entries.push(HistoryEntry {
                journal: path,
                modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                bytes: metadata.len(),
                saved,
            });
        }
        entries.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| b.journal.cmp(&a.journal))
        });
        index::store(self, relative, directory_modified, &entries);
        Ok(entries)
    }

    pub fn read_history(&self, relative: &Path, journal: &Path) -> Result<Recovery, VaultError> {
        Self::validate_relative(relative)?;
        if journal.parent() != Some(self.recovery_dir.as_path())
            || !journal
                .extension()
                .is_some_and(|e| e == "json" || e == "saved")
        {
            return Err(VaultError::InvalidPath);
        }
        // A save can finish after the browser has enumerated its pending journal.
        let saved = journal.with_extension("saved");
        let path = match fs::symlink_metadata(journal) {
            Ok(_) => journal,
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && journal.extension().is_some_and(|e| e == "json") =>
            {
                &saved
            }
            Err(e) => return Err(e.into()),
        };
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_file() || is_reparse(&meta) {
            return Err(VaultError::InvalidPath);
        }
        let record: Recovery = serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| VaultError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
        if record.root != self.root || record.relative != relative {
            return Err(VaultError::InvalidPath);
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_retention_preserves_unlimited_history_and_latest_version() {
        let root = std::env::temp_dir().join(format!("inkstone-retention-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let prefs = vault.root.join(".inkstone-workspace.json");
        fs::write(&prefs, r#"{"history":{"days":0,"max_mib":0}}"#).unwrap();
        let note = Path::new("note.md");
        let first = "a".repeat(600_000);
        let second = "b".repeat(600_000);
        let third = "c".repeat(600_000);
        vault.save(note, None, &first).unwrap();
        vault.save(note, Some(&first), &second).unwrap();
        vault.save(note, Some(&second), &third).unwrap();
        vault.cleanup_history().unwrap();
        assert_eq!(vault.history(note).unwrap().len(), 3);
        fs::write(&prefs, r#"{"history":{"days":0,"max_mib":1}}"#).unwrap();
        vault.cleanup_history().unwrap();
        let entries = vault.history(note).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            vault.read_history(note, &entries[0].journal).unwrap().draft,
            third
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn conflict_resolution_preserves_local_and_rechecks_reviewed_disk() {
        let root = std::env::temp_dir().join(format!("inkstone-conflict-review-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let note = Path::new("note.md");
        vault.save(note, None, "baseline").unwrap();
        fs::write(vault.root.join(note), "external").unwrap();
        assert!(
            vault
                .resolve_conflict(
                    note,
                    Some("baseline"),
                    "local😀",
                    Some("stale"),
                    Some("merged")
                )
                .is_err()
        );
        assert_eq!(vault.read(note).unwrap().as_deref(), Some("external"));
        let text = vault
            .resolve_conflict(note, Some("baseline"), "local😀", Some("external"), None)
            .unwrap();
        assert_eq!(text, "external");
        assert!(
            vault
                .recoveries()
                .unwrap()
                .iter()
                .any(|e| e.record.draft == "local😀")
        );
        assert_eq!(
            vault
                .resolve_conflict(
                    note,
                    Some("external"),
                    "local😀",
                    Some("external"),
                    Some("merged\r\n中😀")
                )
                .unwrap(),
            "merged\r\n中😀"
        );
        assert_eq!(vault.read(note).unwrap().as_deref(), Some("merged\r\n中😀"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn history_lists_all_versions_and_isolates_notes_vaults_and_invalid_records() {
        let root = std::env::temp_dir().join(format!("inkstone-history-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let other = Vault::open(root.join("other"), root.join("recovery")).unwrap();
        let note = Path::new("笔记.md");
        let first = vault.save(note, None, "一\r\n😀").unwrap();
        let second = vault.save(note, Some("一\r\n😀"), "二\r\n😀").unwrap();
        let pending = vault.journal(note, Some("二\r\n😀"), "草稿").unwrap();
        let unrelated = other.save(note, None, "other vault").unwrap();
        vault
            .save(Path::new("别的.md"), None, "other note")
            .unwrap();
        fs::write(vault.recovery_dir.join("broken.json"), "{").unwrap();
        let entries = vault.history(note).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.iter().filter(|e| e.saved).count(), 2);
        assert!(entries.windows(2).all(|w| w[0].modified >= w[1].modified));
        let read = vault.read_history(note, &second.recovery).unwrap();
        assert_eq!(read.baseline.as_deref(), Some("一\r\n😀"));
        assert_eq!(read.draft, "二\r\n😀");
        assert!(vault.read_history(note, &unrelated.recovery).is_err());
        assert!(
            vault
                .read_history(Path::new("别的.md"), &first.recovery)
                .is_err()
        );
        assert!(
            vault
                .read_history(note, &root.join("outside.json"))
                .is_err()
        );
        assert!(vault.history(Path::new("../笔记.md")).is_err());
        fs::rename(&pending, pending.with_extension("saved")).unwrap();
        assert_eq!(vault.read_history(note, &pending).unwrap().draft, "草稿");
        assert_eq!(vault.read(note).unwrap().as_deref(), Some("二\r\n😀"));
        fs::remove_dir_all(root).unwrap();
    }
}
