//! Optional trash presentation metadata, separate from restore authorization.
use super::*;

#[derive(Clone, Debug, Default)]
pub struct TrashMetadata {
    pub modified: Option<SystemTime>,
    /// Logical bytes in regular files; excludes directory metadata/link targets.
    /// None means the traversal failed or overflowed, never a misleading zero.
    pub bytes: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct TrashRecord {
    pub entry: TrashEntry,
    pub metadata: TrashMetadata,
}
impl Vault {
    /// Call off the UI thread. Restore itself uses the cheaper trash_entries API.
    pub fn trash_inventory(&self) -> Result<Vec<TrashRecord>, VaultError> {
        Ok(self
            .trash_entries()?
            .into_iter()
            .map(|entry| {
                let modified = entry
                    .stored
                    .parent()
                    .and_then(|p| fs::symlink_metadata(p.join("original-path.json")).ok())
                    .filter(|m| m.is_file() && !is_reparse(m))
                    .and_then(|m| m.modified().ok());
                let bytes = logical_bytes(&entry.stored).ok();
                TrashRecord {
                    entry,
                    metadata: TrashMetadata { modified, bytes },
                }
            })
            .collect())
    }
}
fn logical_bytes(root: &Path) -> io::Result<u64> {
    let mut pending = vec![root.to_owned()];
    let mut total = 0u64;
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if is_reparse(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| io::Error::other("trash size overflow"))?;
        } else {
            return Err(io::Error::other("unsupported trash file type"));
        }
    }
    Ok(total)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trash_metadata_counts_nested_files_without_following_links() {
        let root = std::env::temp_dir().join(format!("inkstone-trash-metadata-{}", unique_id()));
        fs::create_dir_all(root.join("vault/folder/nested")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        fs::write(vault.root.join("folder/a.md"), "abc").unwrap();
        fs::write(vault.root.join("folder/nested/b.bin"), [0, 1, 2, 3, 4, 5]).unwrap();
        #[cfg(unix)]
        {
            fs::write(root.join("outside"), "must not be counted").unwrap();
            std::os::unix::fs::symlink(root.join("outside"), vault.root.join("folder/link"))
                .unwrap();
        }
        vault.trash_folder(Path::new("folder")).unwrap();
        let records = vault.trash_inventory().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].metadata.bytes, Some(9));
        let descriptor = records[0]
            .entry
            .stored
            .parent()
            .unwrap()
            .join("original-path.json");
        assert_eq!(
            records[0].metadata.modified,
            fs::metadata(descriptor).unwrap().modified().ok()
        );
        assert!(logical_bytes(&root.join("missing")).is_err());
        vault.restore_trash(&records[0].entry).unwrap();
        assert_eq!(
            fs::read(vault.root.join("folder/nested/b.bin")).unwrap(),
            [0, 1, 2, 3, 4, 5]
        );
        assert!(vault.trash_inventory().unwrap().is_empty());
        fs::create_dir(vault.root.join("empty")).unwrap();
        vault.trash_folder(Path::new("empty")).unwrap();
        assert_eq!(vault.trash_inventory().unwrap()[0].metadata.bytes, Some(0));
        fs::remove_dir_all(root).unwrap();
    }
}
