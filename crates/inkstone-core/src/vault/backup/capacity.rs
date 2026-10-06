//! Metadata-only backup capacity. This is not a content integrity verdict.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub directory: PathBuf,
    pub created: u64,
    pub source_name: String,
    pub source_id: Option<String>,
    pub files: usize,
    /// Logical payload bytes, excluding filesystem allocation and extra root files.
    pub payload_bytes: u64,
    pub manifest_bytes: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub entries: Vec<Summary>,
    pub payload_bytes: u64,
    pub manifest_bytes: u64,
    /// Invalid backups, unreadable children and refused links; excluded from totals.
    pub unreadable: usize,
}

/// Validate manifest, layout, types and lengths without reading payload bodies.
/// Restoration must still use `inspect` to verify all content hashes.
pub fn summarize(source: &Path) -> io::Result<Summary> {
    let manifest = inspect_metadata(source)?;
    Ok(Summary {
        directory: source.to_owned(),
        created: manifest.created,
        files: manifest.files.len(),
        payload_bytes: manifest.bytes(),
        source_name: manifest.source_name,
        source_id: manifest.source_id,
        manifest_bytes: fs::metadata(safe_path(source, Path::new("manifest.json"))?)?.len(),
    })
}

/// List immediate backup children of a user-selected backup destination.
/// Unrelated directories and unfinished staging directories are not backups.
pub fn list(directory: &Path) -> io::Result<Inventory> {
    let meta = fs::symlink_metadata(directory)?;
    if !meta.is_dir() || is_reparse(&meta) {
        return Err(invalid("备份位置必须是普通目录"));
    }
    let mut result = Inventory::default();
    for item in fs::read_dir(directory)? {
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                result.unreadable += 1;
                continue;
            }
        };
        let path = item.path();
        let name = item.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => {
                result.unreadable += 1;
                continue;
            }
        };
        if is_reparse(&metadata) {
            result.unreadable += 1;
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let recognized = match fs::symlink_metadata(path.join("manifest.json")) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                name.to_string_lossy().starts_with("inkstone-backup-")
            }
            Err(_) => {
                result.unreadable += 1;
                continue;
            }
        };
        if !recognized {
            continue;
        }
        match summarize(&path) {
            Ok(summary) => {
                let payload = result
                    .payload_bytes
                    .checked_add(summary.payload_bytes)
                    .ok_or_else(|| invalid("备份总大小溢出"))?;
                let manifests = result
                    .manifest_bytes
                    .checked_add(summary.manifest_bytes)
                    .ok_or_else(|| invalid("备份总大小溢出"))?;
                result.payload_bytes = payload;
                result.manifest_bytes = manifests;
                result.entries.push(summary);
            }
            Err(_) => result.unreadable += 1,
        }
    }
    result.entries.sort_by(|a, b| {
        b.created
            .cmp(&a.created)
            .then_with(|| a.directory.cmp(&b.directory))
    });
    Ok(result)
}
