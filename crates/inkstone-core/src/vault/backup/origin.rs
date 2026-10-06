//! Conservative source grouping for backup retention; no note contents are used.
use super::*;

pub(super) fn id(root: &Path) -> io::Result<Option<String>> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() || is_reparse(&meta) {
        return Err(invalid("备份来源必须是普通目录"));
    }
    // Without creation time, inode reuse could group a replacement library with
    // its predecessor. Such sources remain unclassified and protected.
    let Ok(created) = meta.created() else {
        return Ok(None);
    };
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(windows)]
    let identity = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(root)?;
        let (volume, high, low) = SaveGuard::identity(&file)?;
        (u64::from(volume), (u64::from(high) << 32) | u64::from(low))
    };
    let mut hash = Sha256::new();
    hash.update(root.as_os_str().as_encoded_bytes());
    hash.update(serde_json::to_vec(&(identity, created))?);
    Ok(Some(format!("{:x}", hash.finalize())))
}
