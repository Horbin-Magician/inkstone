//! Operation-scoped OS locks, released even when another process inherited the handle.
use std::fs::{File, TryLockError};

/// The underlying handle cannot escape or be cloned through this guard.
#[derive(Debug)]
pub(super) struct FileLock(File);

impl FileLock {
    pub fn acquire(file: File, exclusive: bool) -> Result<Self, TryLockError> {
        if exclusive {
            file.try_lock()?;
        } else {
            file.try_lock_shared()?;
        }
        Ok(Self(file))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Closing alone can leave the lock held by an unrelated forked child
        // until it execs. End ownership explicitly; close remains the fallback.
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    #[test]
    fn releasing_one_reader_keeps_other_reader_and_rejected_writer_protected() {
        let path =
            std::env::temp_dir().join(format!("inkstone-file-lock-{}", super::super::unique_id()));
        let open = || {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap()
        };
        let first = FileLock::acquire(open(), false).unwrap();
        let second = FileLock::acquire(open(), false).unwrap();
        assert!(matches!(
            FileLock::acquire(open(), true),
            Err(TryLockError::WouldBlock)
        ));
        drop(first);
        assert!(matches!(
            FileLock::acquire(open(), true),
            Err(TryLockError::WouldBlock)
        ));
        drop(second);
        let writer = FileLock::acquire(open(), true).unwrap();
        assert!(matches!(
            FileLock::acquire(open(), false),
            Err(TryLockError::WouldBlock)
        ));
        drop(writer);
        drop(FileLock::acquire(open(), false).unwrap());
        std::fs::remove_file(path).unwrap();
    }
}
