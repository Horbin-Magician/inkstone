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
pub(super) fn read_primary(vault: &Vault) -> Result<Option<Links>, VaultError> {
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
pub(super) fn proof_path(vault: &Vault) -> PathBuf {
    path(vault).with_extension("history-proof")
}
fn regular_bytes(path: &Path) -> Result<Vec<u8>, VaultError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || is_reparse(&meta) {
        return Err(invalid());
    }
    Ok(fs::read(path)?)
}
pub(super) fn read(vault: &Vault) -> Result<Option<Links>, VaultError> {
    let primary = read_primary(vault);
    if matches!(primary, Ok(Some(_))) {
        return primary;
    }
    // Only a separately certified mirror may replace missing/corrupt primary data.
    // Do not rewrite the primary here: a concurrent rename may be publishing it.
    let proof = match regular_bytes(&proof_path(vault)) {
        Ok(proof) => proof,
        Err(_) => return primary,
    };
    let links: Links =
        serde_json::from_slice(&regular_bytes(&super::path(vault))?).map_err(|_| invalid())?;
    if checksum(&links)?.as_bytes() != proof {
        return Err(invalid());
    }
    Ok(Some(links))
}
/// Called under the rename lock before any file move. An old certificate must
/// never authorize the old mirror after a newer checkpoint publication fails.
pub(super) fn invalidate_proof(vault: &Vault) -> Result<Option<Vec<u8>>, VaultError> {
    let path = proof_path(vault);
    let previous = match regular_bytes(&path) {
        Ok(bytes) => Some(bytes),
        Err(VaultError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    replace_proof(vault, b"pending")?;
    Ok(previous)
}
fn replace_proof(vault: &Vault, bytes: &[u8]) -> Result<(), VaultError> {
    let proof = proof_path(vault);
    let temp = proof.with_extension(format!("{}.proof-temp", unique_id()));
    let result = write_new_synced(&temp, bytes).and_then(|()| fs::rename(&temp, proof));
    let _ = fs::remove_file(temp);
    Ok(result?)
}
pub(super) fn restore_proof(vault: &Vault, previous: Option<Vec<u8>>) {
    if let Some(bytes) = previous {
        let _ = replace_proof(vault, &bytes);
    } else {
        let _ = fs::remove_file(proof_path(vault));
    }
}
/// Called only by the successful publisher under the rename lock.
pub(super) fn certify(vault: &Vault, links: &Links) {
    let Ok(expected) = checksum(links) else {
        return;
    };
    let Ok(mirror) = regular_bytes(&super::path(vault)) else {
        return;
    };
    let Ok(mirror) = serde_json::from_slice::<Links>(&mirror) else {
        return;
    };
    if checksum(&mirror).ok().as_deref() != Some(expected.as_str()) {
        return;
    }
    let proof = proof_path(vault);
    let temp = proof.with_extension(format!("{}.proof-temp", unique_id()));
    if write_new_synced(&temp, expected.as_bytes()).is_ok() {
        let _ = fs::rename(&temp, proof);
    }
    let _ = fs::remove_file(temp);
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
        // The certified intact mirror keeps history available.
        assert_eq!(vault.history(new).unwrap().len(), 2);
        // A stale but valid JSON mirror cannot satisfy the current certificate.
        fs::write(&mirror, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(vault.history(new).is_err());
        assert!(
            vault
                .rename_note(new, Path::new("third.md"), "body")
                .is_err()
        );
        assert_eq!(vault.read(new).unwrap().as_deref(), Some("body"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn certified_mirror_survives_missing_primary_and_failed_next_publication() {
        let root = std::env::temp_dir().join(format!("inkstone-links-proof-{}", unique_id()));
        fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let a = Path::new("a.md");
        let b = Path::new("b.md");
        let c = Path::new("c.md");
        let first = vault.save(a, None, "body").unwrap();
        vault.rename_note(a, b, "body").unwrap();
        assert!(proof_path(&vault).is_file());
        fs::remove_file(path(&vault)).unwrap();
        assert!(vault.read_history(b, &first.recovery).is_ok());
        vault.rename_note(b, c, "body").unwrap();
        assert!(path(&vault).is_file());
        let before = fs::read(path(&vault)).unwrap();
        assert!(
            super::super::rename_with(
                &vault,
                c,
                a,
                &vault.root.join(c),
                &vault.root.join(a),
                |_, _| {
                    assert_eq!(fs::read(proof_path(&vault)).unwrap(), b"pending");
                    Err(io::Error::other("publication failed"))
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(path(&vault)).unwrap(), before);
        assert!(vault.read_history(c, &first.recovery).is_ok());
        assert!(vault.read(c).unwrap().is_some());
        fs::write(path(&vault), b"damaged").unwrap();
        assert!(
            vault.history(c).is_ok(),
            "rollback restores the previous certificate"
        );
        replace_proof(&vault, b"pending").unwrap();
        assert!(
            vault.history(c).is_err(),
            "an interrupted publication cannot trust an old mirror"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
