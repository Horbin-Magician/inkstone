//! Conservative local-filesystem gate for destructive backup cleanup.
use super::*;

#[cfg(target_os = "macos")]
pub(super) fn local(path: &Path) -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    let path = fs::canonicalize(path)?;
    let path =
        std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("无效存储路径"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // statfs initializes the output only on success; path is NUL terminated.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    Ok(apple_local(stat.f_flags))
}
#[cfg(target_os = "macos")]
fn apple_local(flags: u32) -> bool {
    flags & libc::MNT_LOCAL as u32 != 0
}

#[cfg(target_os = "linux")]
pub(super) fn local(path: &Path) -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    let path = fs::canonicalize(path)?;
    let path =
        std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("无效存储路径"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    // ext2/3/4, XFS, Btrfs and tmpfs. Unknown/FUSE/overlay/network mounts
    // remain unsupported rather than inferring the underlying storage.
    Ok(matches!(
        stat.f_type as u64,
        0xef53 | 0x58465342 | 0x9123683e | 0x01021994
    ))
}

#[cfg(windows)]
pub(super) fn local(path: &Path) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    let path = fs::canonicalize(path)?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // Removable, fixed or RAM disk; never remote/unknown/no-root/CD-ROM.
    Ok(matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 6))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(super) fn local(path: &Path) -> io::Result<bool> {
    fs::canonicalize(path)?;
    Ok(false)
}

pub(in crate::vault) fn require_local(path: &Path) -> io::Result<()> {
    if !local(path)? {
        return Err(invalid(
            "此位置不是受支持的本地存储，已禁止清理；仍可查看和恢复备份",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_storage_is_an_error() {
        assert!(
            local(&std::env::temp_dir().join(format!("inkstone-missing-{}", unique_id()))).is_err()
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_mount_flags_require_explicit_local_bit() {
        assert!(!apple_local(0));
        assert!(!apple_local(libc::MNT_RDONLY as u32));
        assert!(apple_local(libc::MNT_LOCAL as u32));
        assert!(apple_local((libc::MNT_LOCAL | libc::MNT_RDONLY) as u32));
        assert!(local(&std::env::temp_dir()).unwrap());
    }
}
