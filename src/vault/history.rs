use super::*;

/// Metadata only: selecting an entry loads its body on a background thread.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub journal: PathBuf,
    pub modified: SystemTime,
    pub bytes: u64,
    pub saved: bool,
}

impl Vault {
    pub fn history(&self, relative: &Path) -> Result<Vec<HistoryEntry>, VaultError> {
        Self::validate_relative(relative)?;
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
            if self.read_history(relative, &path).is_err() {
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
