//! Checksummed authoritative checkpoint; the older map is a repairable mirror.
use super::*;

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    links: Links,
    checksum: String,
}
pub(super) fn path(vault: &Vault) -> PathBuf {
    super::path(vault).with_extension("history-commit")
}
fn checksum(links: &Links) -> Result<String, VaultError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(links).map_err(io::Error::other)?)
    ))
}
pub(super) fn encode(links: &Links) -> Result<Vec<u8>, VaultError> {
    // Borrow the full map during serialization rather than clone its entries.
    #[derive(Serialize)]
    struct Borrowed<'a> {
        version: u32,
        links: &'a Links,
        checksum: String,
    }
    serde_json::to_vec(&Borrowed {
        version: 1,
        links,
        checksum: checksum(links)?,
    })
    .map_err(|e| io::Error::other(e).into())
}
pub(super) fn read(vault: &Vault) -> Result<Option<Links>, VaultError> {
    let path = path(vault);
    let meta = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(meta) => meta,
    };
    if !meta.is_file() || is_reparse(&meta) {
        return Err(invalid());
    }
    let checkpoint: Checkpoint = serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid())?;
    if checkpoint.version != 1 || checksum(&checkpoint.links)? != checkpoint.checksum {
        return Err(invalid());
    }
    Ok(Some(checkpoint.links))
}
pub(super) fn repair_cache(vault: &Vault, links: &Links) {
    let path = super::path(vault);
    let Ok(bytes) = serde_json::to_vec(links) else {
        return;
    };
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || is_reparse(&meta) => return,
        Ok(_) if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) => return,
        _ => {}
    }
    let temp = path.with_extension(format!("{}.cache-temp", unique_id()));
    if write_new_synced(&temp, &bytes).is_ok() {
        let _ = fs::rename(&temp, path);
    }
    let _ = fs::remove_file(temp);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_and_corrupt_mirror_repair_preserve_latest_ownership() {
        let root = std::env::temp_dir().join(format!("inkstone-links-checkpoint-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = Path::new("old.md");
        let new = Path::new("new.md");
        let first = vault.save(old, None, "body").unwrap();
        let legacy = Links {
            version: 1,
            root: vault.root.clone(),
            owners: BTreeMap::new(),
        };
        fs::write(
            super::super::path(&vault),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        assert_eq!(vault.history(old).unwrap().len(), 1);
        vault.rename_note(old, new, "body").unwrap();
        assert!(path(&vault).is_file());
        let mirror = super::super::path(&vault);
        for damaged in [b"broken".to_vec(), serde_json::to_vec(&legacy).unwrap()] {
            fs::write(&mirror, damaged).unwrap();
            assert_eq!(
                vault.read_history(new, &first.recovery).unwrap().draft,
                "body"
            );
            let repaired: Links = serde_json::from_slice(&fs::read(&mirror).unwrap()).unwrap();
            assert_eq!(repaired.owners[&first.recovery.with_extension("")], new);
            assert!(vault.history(old).unwrap().is_empty());
        }
        fs::remove_file(&mirror).unwrap();
        assert_eq!(vault.history(new).unwrap().len(), 2);
        assert!(mirror.is_file());
        // A damaged authoritative checkpoint cannot silently fall back to a stale mirror.
        let mut damaged: serde_json::Value =
            serde_json::from_slice(&fs::read(path(&vault)).unwrap()).unwrap();
        damaged["links"]["owners"] = serde_json::json!({});
        fs::write(path(&vault), serde_json::to_vec(&damaged).unwrap()).unwrap();
        assert!(vault.history(new).is_err());
        assert!(
            vault
                .rename_note(new, Path::new("third.md"), "body")
                .is_err()
        );
        assert_eq!(vault.read(new).unwrap().as_deref(), Some("body"));
        fs::remove_dir_all(root).unwrap();
    }
}
