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
        // Validate the complete legacy record, but retain only its ownership.
        // The JSON decoder reuses scratch space instead of keeping both bodies.
        let file = fs::File::open(path).ok()?;
        let metadata = serde_json::from_reader::<_, super::metadata::Metadata>(
            io::BufReader::with_capacity(64 * 1024, file),
        )
        .ok()?;
        Some(Scope {
            root: metadata.root,
            relative: metadata.relative,
        })
    })
}
fn scope_with(
    vault: &Vault,
    path: &Path,
    meta: &fs::Metadata,
    read: impl FnOnce() -> Option<Scope>,
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
    let scope = read()?;
    let current = fs::symlink_metadata(path).ok()?;
    if !current.is_file()
        || is_reparse(&current)
        || current.len() != meta.len()
        || current.modified().ok()? != modified
    {
        return None;
    }
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
    #[test]
    fn cold_import_validates_large_legacy_records_and_rebuilds_metadata() {
        let root = std::env::temp_dir().join(format!("inkstone-cold-history-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let path = vault.recovery_dir.join("legacy.saved");
        let note = Path::new("旧目录/中文😀.md");
        let record = Recovery {
            root: vault.root.clone(),
            relative: note.into(),
            baseline: Some("旧正文\n\"\\中文😀".repeat(40_000)),
            draft: "新正文\r\n\"\\é😀".repeat(40_000),
        };
        // Direct legacy publication intentionally bypasses the metadata seed.
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let meta = fs::symlink_metadata(&path).unwrap();
        assert!(meta.len() > 1024 * 1024);
        let imported = scope(&vault, &path, &meta).unwrap();
        assert_eq!(imported.root, vault.root);
        assert_eq!(imported.relative, note);
        assert_eq!(vault.read_history(note, &path).unwrap(), record);
        let cache = cache_path(&vault, &path.with_extension("")).unwrap();
        assert!(fs::metadata(&cache).unwrap().len() < 2048);
        assert!(
            scope_with(&vault, &path, &meta, || panic!(
                "warm lookup must not parse bodies"
            ))
            .is_some()
        );
        // Complete metadata must not conceal a malformed body or trailing input.
        for invalid in [
            serde_json::json!({"root": vault.root, "relative": note, "baseline": null, "draft": [1]}).to_string(),
            format!("{} trailing", serde_json::to_string(&record).unwrap()),
            serde_json::to_string(&record).unwrap().trim_end_matches('}').to_owned(),
        ] {
            fs::write(&path, invalid).unwrap();
            fs::remove_file(&cache).unwrap();
            assert!(scope(&vault, &path, &fs::symlink_metadata(&path).unwrap()).is_none());
            // Restore a valid cold import before checking the next corruption.
            fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            assert!(scope(&vault, &path, &fs::symlink_metadata(&path).unwrap()).is_some());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_decoder_matches_legacy_schema_and_string_validation() {
        let cases: &[&[u8]] = &[
            br#"{"root":"/vault","relative":"note.md","draft":""}"#,
            br#"{"root":"/vault","relative":"note.md","baseline":null,"draft":"new"}"#,
            br#"{"root":"/vault","relative":"note.md","baseline":"old","draft":"\u4e2d\ud83d\ude00\n\"\\"}"#,
            br#"{"draft":"new","unknown":{"nested":[1,true,null]},"relative":"note.md","root":"/vault"}"#,
            br#"{"root":"/vault","relative":"note.md"}"#,
            br#"{"root":"/vault","draft":"new"}"#,
            br#"{"relative":"note.md","draft":"new"}"#,
            br#"{"root":"/vault","relative":"note.md","draft":null}"#,
            br#"{"root":"/vault","relative":"note.md","draft":42}"#,
            br#"{"root":"/vault","relative":"note.md","draft":[]}"#,
            br#"{"root":"/vault","relative":"note.md","draft":{}}"#,
            br#"{"root":"/vault","relative":"note.md","baseline":false,"draft":"new"}"#,
            br#"{"root":"/vault","relative":"note.md","baseline":["old"],"draft":"new"}"#,
            br#"{"root":"/vault","relative":"note.md","draft":"\ud800"}"#,
            br#"{"root":"/vault","relative":"note.md","draft":"\udc00"}"#,
            br#"{"root":"/vault","relative":"note.md","draft":"\q"}"#,
            b"{\"root\":\"/vault\",\"relative\":\"note.md\",\"draft\":\"\xff\"}",
            b"{\"root\":\"/vault\",\"relative\":\"note.md\",\"draft\":\"\n\"}",
            br#"{"root":"/vault","relative":"note.md","draft":"one","draft":"two"}"#,
            br#"{"root":"/vault","relative":"note.md","baseline":null,"baseline":"old","draft":"new"}"#,
            br#"{"root":"/vault","relative":"note.md","draft":"new"} trailing"#,
            br#"{"root":"/vault","relative":"note.md","draft":"truncated"#,
        ];
        for input in cases {
            let original = serde_json::from_reader::<_, Recovery>(*input);
            let metadata = serde_json::from_reader::<_, super::super::metadata::Metadata>(*input);
            assert_eq!(original.is_ok(), metadata.is_ok(), "{input:?}");
            if let (Ok(original), Ok(metadata)) = (original, metadata) {
                assert_eq!(original.root, metadata.root);
                assert_eq!(original.relative, metadata.relative);
            }
        }
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
