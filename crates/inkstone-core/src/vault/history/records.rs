//! Reusable journal metadata, including other notes/vaults, without caching bodies.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Scope {
    pub root: PathBuf,
    pub relative: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    identity: PathBuf,
    bytes: u64,
    modified: SystemTime,
    scope: Scope,
}
#[derive(Serialize, Deserialize)]
struct Cache {
    record: Record,
    checksum: String,
}
fn checksum(record: &Record) -> Option<String> {
    Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(record).ok()?)
    ))
}
fn cache_path(vault: &Vault, identity: &Path) -> Option<PathBuf> {
    let directory = super::index::directory(vault)?.join("records");
    fs::create_dir_all(&directory).ok()?;
    let meta = fs::symlink_metadata(&directory).ok()?;
    if !meta.is_dir() || is_reparse(&meta) {
        return None;
    }
    let key = serde_json::to_vec(identity).ok()?;
    Some(directory.join(format!("{:x}.json", Sha256::digest(key))))
}
pub(super) fn scope(vault: &Vault, path: &Path, meta: &fs::Metadata) -> Option<Scope> {
    scope_with(vault, path, meta, || {
        serde_json::from_slice::<Recovery>(&fs::read(path).ok()?).ok()
    })
}
fn scope_with(
    vault: &Vault,
    path: &Path,
    meta: &fs::Metadata,
    read: impl FnOnce() -> Option<Recovery>,
) -> Option<Scope> {
    if !meta.is_file() || is_reparse(meta) {
        return None;
    }
    // Journal publication changes json to saved without changing its identity/body.
    let identity = path.with_extension("");
    let modified = meta.modified().ok()?;
    let cache_path = cache_path(vault, &identity);
    if let Some(cache) = cache_path.as_ref().and_then(|path| {
        let meta = fs::symlink_metadata(path).ok()?;
        if !meta.is_file() || is_reparse(&meta) {
            return None;
        }
        serde_json::from_slice::<Cache>(&fs::read(path).ok()?).ok()
    }) && cache.record.version == 1
        && cache.record.identity == identity
        && cache.record.bytes == meta.len()
        && cache.record.modified == modified
        && checksum(&cache.record).as_deref() == Some(cache.checksum.as_str())
    {
        return Some(cache.record.scope);
    }
    let recovery = read()?;
    let current = fs::symlink_metadata(path).ok()?;
    if !current.is_file()
        || is_reparse(&current)
        || current.len() != meta.len()
        || current.modified().ok()? != modified
    {
        return None;
    }
    let scope = Scope {
        root: recovery.root,
        relative: recovery.relative,
    };
    store(cache_path, identity, meta.len(), modified, scope.clone());
    Some(scope)
}

/// Best effort: journal durability and saving never depend on this cache.
pub(super) fn seed(vault: &Vault, path: &Path, recovery: &Recovery) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_file() || is_reparse(&meta) {
        return;
    }
    let Ok(modified) = meta.modified() else {
        return;
    };
    let identity = path.with_extension("");
    store(
        cache_path(vault, &identity),
        identity,
        meta.len(),
        modified,
        Scope {
            root: recovery.root.clone(),
            relative: recovery.relative.clone(),
        },
    );
}

fn store(path: Option<PathBuf>, identity: PathBuf, bytes: u64, modified: SystemTime, scope: Scope) {
    let record = Record {
        version: 1,
        identity,
        bytes,
        modified,
        scope,
    };
    if let Some(path) = path
        && let Some(checksum) = checksum(&record)
        && let Ok(bytes) = serde_json::to_vec(&Cache { record, checksum })
    {
        let temp = path.with_extension(format!("{}.tmp", unique_id()));
        if write_new_synced(&temp, &bytes).is_ok() {
            let _ = fs::rename(&temp, &path);
        }
        let _ = fs::remove_file(temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_reuses_old_journals_and_publication_without_reading_bodies() {
        let root = std::env::temp_dir().join(format!("inkstone-history-records-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let note = Path::new("old.md");
        let journal = vault.journal(note, None, &"body".repeat(250_000)).unwrap();
        // No history request has read or imported this journal yet.
        assert!(
            scope_with(
                &vault,
                &journal,
                &fs::symlink_metadata(&journal).unwrap(),
                || panic!("new journal metadata must already exist")
            )
            .is_some()
        );
        assert_eq!(vault.history(note).unwrap().len(), 1);
        vault.journal(Path::new("new.md"), None, "new").unwrap();
        let meta = fs::symlink_metadata(&journal).unwrap();
        assert_eq!(
            scope_with(&vault, &journal, &meta, || panic!(
                "cached body must not be read"
            ))
            .unwrap()
            .relative,
            note
        );
        let saved = journal.with_extension("saved");
        fs::rename(&journal, &saved).unwrap();
        assert_eq!(
            scope_with(
                &vault,
                &saved,
                &fs::symlink_metadata(&saved).unwrap(),
                || panic!("publication preserves metadata")
            )
            .unwrap()
            .relative,
            note
        );
        assert_eq!(vault.history(note).unwrap().len(), 1);
        let cache = cache_path(&vault, &saved.with_extension("")).unwrap();
        fs::write(&cache, b"broken").unwrap();
        assert!(scope_with(&vault, &saved, &meta, || None).is_none());
        assert!(scope(&vault, &saved, &meta).is_some());
        fs::write(&saved, b"broken journal").unwrap();
        assert!(scope(&vault, &saved, &fs::symlink_metadata(&saved).unwrap()).is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn linked_record_cache_is_ignored_and_vault_scope_is_preserved() {
        let root = std::env::temp_dir().join(format!("inkstone-record-link-{}", unique_id()));
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        fs::create_dir_all(root.join("outside")).unwrap();
        let one = Vault::open(root.join("one"), root.join("recovery")).unwrap();
        let two = Vault::open(root.join("two"), root.join("recovery")).unwrap();
        let note = Path::new("same.md");
        one.journal(note, None, "one").unwrap();
        two.journal(note, None, "two").unwrap();
        let directory = super::super::index::directory(&one).unwrap();
        fs::remove_dir_all(directory.join("records")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), directory.join("records")).unwrap();
        one.journal(Path::new("fallback.md"), None, "cache failure is harmless")
            .unwrap();
        for vault in [&one, &two] {
            let entries = vault.history(note).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(
                vault.read_history(note, &entries[0].journal).unwrap().root,
                vault.root
            );
        }
        assert_eq!(fs::read_dir(root.join("outside")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
