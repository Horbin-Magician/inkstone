//! Library-wide discovery also exposes history whose note no longer exists.
use super::*;
use std::collections::BTreeMap;

/// Summary of the recorded owner, not a claim that the note still exists there.
/// External renames cannot be inferred safely from equal names or contents.
#[derive(Clone, Debug)]
pub struct HistoryNote {
    pub relative: PathBuf,
    pub records: usize,
    pub bytes: u64,
    pub modified: SystemTime,
}

impl Vault {
    /// Discover note history without requiring an open/existing Markdown file.
    /// Warm calls read cached scope metadata; bodies are loaded only on selection.
    /// Legacy/corrupt-cache cold imports still validate each journal's JSON.
    pub fn history_notes(&self) -> Result<Vec<HistoryNote>, VaultError> {
        let ownership = links::Ownership::load(self)?;
        let mut notes = BTreeMap::<PathBuf, HistoryNote>::new();
        for item in fs::read_dir(&self.recovery_dir)? {
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
            if !meta.is_file() || is_reparse(&meta) {
                continue;
            }
            let Some(scope) = records::scope(self, &journal, &meta) else {
                continue;
            };
            if scope.root != self.root {
                continue;
            }
            let relative = ownership.owner(&journal, &scope.relative);
            if Self::validate_relative(relative).is_err() {
                continue;
            }
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            let entry = notes
                .entry(relative.to_owned())
                .or_insert_with(|| HistoryNote {
                    relative: relative.to_owned(),
                    records: 0,
                    bytes: 0,
                    modified,
                });
            entry.records += 1;
            entry.bytes = entry.bytes.saturating_add(meta.len());
            entry.modified = entry.modified.max(modified);
        }
        // Stable path order makes a library browser independent of filesystem order.
        Ok(notes.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_external_rename_and_deletion_without_guessing_new_identity() {
        let root = std::env::temp_dir().join(format!("inkstone-history-catalog-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = Path::new("旧名字.md");
        vault.save(old, None, "第一版").unwrap();
        vault.save(old, Some("第一版"), "第二版").unwrap();
        fs::rename(vault.root.join(old), vault.root.join("外部重命名.md")).unwrap();
        vault.save(Path::new("删除.md"), None, "删除前").unwrap();
        fs::remove_file(vault.root.join("删除.md")).unwrap();
        let notes = vault.history_notes().unwrap();
        assert_eq!(notes.len(), 2);
        let note = notes.iter().find(|n| n.relative == old).unwrap();
        let entries = vault.history(old).unwrap();
        assert_eq!(note.records, 2);
        assert_eq!(note.bytes, entries.iter().map(|e| e.bytes).sum::<u64>());
        assert_eq!(
            note.modified,
            entries.iter().map(|e| e.modified).max().unwrap()
        );
        for entry in entries {
            assert_eq!(
                vault.read_history(old, &entry.journal).unwrap().relative,
                old
            );
        }
        assert!(
            vault
                .history(Path::new("外部重命名.md"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            vault.read(Path::new("外部重命名.md")).unwrap().as_deref(),
            Some("第二版")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_uses_rename_ownership_and_excludes_foreign_and_invalid_records() {
        let root =
            std::env::temp_dir().join(format!("inkstone-history-catalog-scope-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        let other = Vault::open(root.join("other"), root.join("recovery")).unwrap();
        let old = Path::new("old.md");
        let new = Path::new("new.md");
        vault.save(old, None, "saved").unwrap();
        vault.rename_note(old, new, "saved").unwrap();
        let pending = vault.journal(new, Some("saved"), "pending").unwrap();
        let mut invalid: serde_json::Value =
            serde_json::from_slice(&fs::read(&pending).unwrap()).unwrap();
        invalid["relative"] = serde_json::json!("../outside.md");
        fs::write(
            vault.recovery_dir.join("invalid.saved"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&pending, vault.recovery_dir.join("linked.json")).unwrap();
        other
            .save(Path::new("foreign.md"), None, "private")
            .unwrap();
        fs::write(vault.recovery_dir.join("broken.saved"), "{").unwrap();
        fs::create_dir(vault.recovery_dir.join("directory.saved")).unwrap();
        for _ in 0..2 {
            let notes = vault.history_notes().unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].relative, new);
            // Initial save, pre-rename snapshot, and pending draft.
            assert_eq!(notes[0].records, 3);
            assert_eq!(vault.history(new).unwrap().len(), notes[0].records);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
