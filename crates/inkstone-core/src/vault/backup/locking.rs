//! Local-user coordination, stored outside backup media so read-only backups work.
use super::*;

pub(super) fn acquire(directory: &Path, exclusive: bool) -> io::Result<fs::File> {
    let directory = fs::canonicalize(directory)?;
    let mut digest = Sha256::new();
    digest.update(directory.as_os_str().as_encoded_bytes());
    #[cfg(unix)]
    let user = unsafe { libc::geteuid() }.to_string();
    #[cfg(windows)]
    let user = "current-user";
    let root = std::env::temp_dir().join(format!("inkstone-backup-locks-{user}"));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let meta = fs::symlink_metadata(&root)?;
    if !meta.is_dir() || is_reparse(&meta) {
        return Err(invalid("备份锁目录无效"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(invalid("备份锁目录权限无效"));
        }
    }
    let path = root.join(format!("{:x}.lock", digest.finalize()));
    if let Ok(meta) = fs::symlink_metadata(&path)
        && (!meta.is_file() || is_reparse(&meta))
    {
        return Err(invalid("备份锁文件无效"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let result = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    result.map_err(|error| match error {
        fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "备份位置有其他备份、校验、恢复或清理任务",
        ),
        fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}
