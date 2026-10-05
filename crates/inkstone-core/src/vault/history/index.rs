//! Disposable per-vault/per-note metadata cache. Journals remain authoritative.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    root: PathBuf,
    relative: PathBuf,
    directory_modified: SystemTime,
    entries: Vec<HistoryEntry>,
}
#[derive(Serialize, Deserialize)]
struct Cache {
    index: Index,
    checksum: String,
}
fn checksum(index: &Index) -> Option<String> {
    Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(index).ok()?)
    ))
}
fn cache_path(vault: &Vault, relative: &Path) -> Option<PathBuf> {
    let directory = vault.recovery_dir.join(".history-index");
    fs::create_dir_all(&directory).ok()?;
    let meta = fs::symlink_metadata(&directory).ok()?;
    if !meta.is_dir() || is_reparse(&meta) {
        return None;
    }
    let key = serde_json::to_vec(&(&vault.root, relative)).ok()?;
    Some(directory.join(format!("{:x}.json", Sha256::digest(key))))
}
pub(super) fn stamp(vault: &Vault) -> Option<SystemTime> {
    fs::metadata(&vault.recovery_dir).ok()?.modified().ok()
}
pub(super) fn load(vault: &Vault, relative: &Path) -> Option<Vec<HistoryEntry>> {
    let path = cache_path(vault, relative)?;
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || is_reparse(&meta) {
        return None;
    }
    let cache: Cache = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if checksum(&cache.index)? != cache.checksum {
        return None;
    }
    let index = cache.index;
    if index.version != 1
        || index.root != vault.root
        || index.relative != relative
        || Some(index.directory_modified) != stamp(vault)
    {
        return None;
    }
    for entry in &index.entries {
        if entry.journal.parent() != Some(vault.recovery_dir.as_path())
            || !entry
                .journal
                .extension()
                .is_some_and(|e| e == "json" || e == "saved")
            || entry.saved != entry.journal.extension().is_some_and(|e| e == "saved")
        {
            return None;
        }
        let meta = fs::symlink_metadata(&entry.journal).ok()?;
        if !meta.is_file()
            || is_reparse(&meta)
            || meta.len() != entry.bytes
            || meta.modified().ok()? != entry.modified
        {
            return None;
        }
    }
    Some(index.entries)
}
pub(super) fn store(
    vault: &Vault,
    relative: &Path,
    before: Option<SystemTime>,
    entries: &[HistoryEntry],
) {
    let Some(path) = cache_path(vault, relative) else {
        return;
    };
    let Some(modified) = before.filter(|time| Some(*time) == stamp(vault)) else {
        return;
    };
    let index = Index {
        version: 1,
        root: vault.root.clone(),
        relative: relative.to_owned(),
        directory_modified: modified,
        entries: entries.to_vec(),
    };
    let Some(checksum) = checksum(&index) else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(&Cache { index, checksum }) else {
        return;
    };
    let temp = path.with_extension(format!("{}.tmp", unique_id()));
    if write_new_synced(&temp, &bytes).is_ok() {
        let _ = fs::rename(&temp, &path);
    }
    let _ = fs::remove_file(temp);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_metadata_rebuilds_after_journal_changes_or_cache_damage() {
        let root = std::env::temp_dir().join(format!("inkstone-history-index-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let note = Path::new("note.md");
        vault
            .save(note, None, &"large body".repeat(100_000))
            .unwrap();
        let entries = vault.history(note).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(load(&vault, note).unwrap()[0].journal, entries[0].journal);
        let cache = cache_path(&vault, note).unwrap();
        assert!(
            fs::metadata(&cache).unwrap().len() < 2048,
            "cache contains no body"
        );
        fs::write(&cache, b"corrupt").unwrap();
        assert!(load(&vault, note).is_none());
        assert_eq!(vault.history(note).unwrap().len(), 1);
        let mut damaged: serde_json::Value =
            serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        damaged["index"]["entries"] = serde_json::json!([]);
        fs::write(&cache, serde_json::to_vec(&damaged).unwrap()).unwrap();
        assert!(
            load(&vault, note).is_none(),
            "valid JSON with damaged metadata also rebuilds"
        );
        assert_eq!(vault.history(note).unwrap().len(), 1);
        let pending = vault.journal(note, None, "new draft").unwrap();
        assert!(load(&vault, note).is_none());
        assert_eq!(vault.history(note).unwrap().len(), 2);
        fs::rename(&pending, pending.with_extension("saved")).unwrap();
        assert_eq!(
            vault
                .history(note)
                .unwrap()
                .iter()
                .filter(|e| e.saved)
                .count(),
            2
        );
        fs::write(&entries[0].journal, b"broken journal body").unwrap();
        assert!(load(&vault, note).is_none());
        assert_eq!(vault.history(note).unwrap().len(), 1);
        assert!(vault.history(Path::new("other.md")).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn unsafe_cache_directory_is_ignored_without_following_it() {
        let root =
            std::env::temp_dir().join(format!("inkstone-history-cache-link-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        fs::create_dir_all(root.join("outside")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        vault.save(Path::new("note.md"), None, "body").unwrap();
        std::os::unix::fs::symlink(
            root.join("outside"),
            vault.recovery_dir.join(".history-index"),
        )
        .unwrap();
        assert_eq!(vault.history(Path::new("note.md")).unwrap().len(), 1);
        assert_eq!(fs::read_dir(root.join("outside")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
