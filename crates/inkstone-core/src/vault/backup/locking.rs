//! Local-user coordination, stored outside backup media so read-only backups work.
use super::*;

use crate::vault::file_lock::FileLock;

pub(super) fn acquire(directory: &Path, exclusive: bool) -> io::Result<FileLock> {
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
    FileLock::acquire(file, exclusive).map_err(|error| match error {
        fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "备份位置有其他备份、校验、恢复或清理任务",
        ),
        fs::TryLockError::Error(error) => error,
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::{net::UnixStream, process::CommandExt},
        },
        process::Command,
        time::Duration,
    };

    #[test]
    fn operation_release_does_not_wait_for_an_unrelated_child_to_exec() {
        let directory = std::env::temp_dir().join(format!("inkstone-lock-exec-{}", unique_id()));
        fs::create_dir(&directory).unwrap();
        let owner = acquire(&directory, true).unwrap();
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        child
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let process = std::thread::spawn(move || {
            let mut command = Command::new("/usr/bin/true");
            // Force the fork/exec interval to remain open while the operation ends.
            // Only async-signal-safe syscalls run in the child before exec.
            unsafe {
                command.pre_exec(move || {
                    let mut byte = 1_u8;
                    if libc::write(child.as_raw_fd(), (&byte as *const u8).cast(), 1) != 1
                        || libc::read(child.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) != 1
                    {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            command.status()
        });
        let ready = parent.read_exact(&mut [0]);
        drop(owner);
        let next = acquire(&directory, true);
        // Unblock and reap even if the assertion will fail.
        let released = parent.write_all(&[1]);
        let status = process.join().unwrap();
        fs::remove_dir(&directory).unwrap();
        ready.unwrap();
        released.unwrap();
        assert!(status.unwrap().success());
        next.expect("a completed operation must release its lock before an unrelated child execs");
    }
}
