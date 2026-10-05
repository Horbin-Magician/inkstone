//! WebDAV passwords stay out of the vault. They are stored in plaintext in
//! the app data directory so opening the app does not prompt for keychain access.
use anyhow::{Context, Result, bail};
use inkstone_core::vault::sync::Settings;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const FILE_NAME: &str = "webdav-secrets.json";

/// Stable credential account. The raw URL, username, and vault path are not stored.
pub(super) fn account(settings: &Settings, vault_root: &Path) -> Result<String> {
    let url = settings.canonical_url()?;
    let username = settings.username.trim();
    if username.is_empty() {
        bail!("请先填写 WebDAV 用户名");
    }
    let material = format!("{url}\n{username}\n{}", vault_root.display());
    Ok(format!("{:x}", Sha256::digest(material.as_bytes())))
}

pub(super) fn load(settings: &Settings, vault_root: &Path) -> Result<Option<String>> {
    if settings.username.trim().is_empty() || settings.url.trim().is_empty() {
        return Ok(None);
    }
    let account = account(settings, vault_root)?;
    Ok(read()?.remove(&account))
}

pub(super) fn store(account: &str, password: &str) -> Result<()> {
    let _guard = lock();
    let mut secrets = read_unlocked()?;
    if password.is_empty() {
        secrets.remove(account);
    } else {
        secrets.insert(account.to_owned(), password.to_owned());
    }
    write(&secrets)
}

fn secrets_path() -> PathBuf {
    super::app_dir().join(FILE_NAME)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn read() -> Result<BTreeMap<String, String>> {
    let _guard = lock();
    read_unlocked()
}

fn read_unlocked() -> Result<BTreeMap<String, String>> {
    let path = secrets_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取已保存的 WebDAV 密码：{}", path.display()));
        }
    };
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("已保存的 WebDAV 密码无法读取：{}", path.display()))?;
    let Value::Object(object) = value else {
        bail!("已保存的 WebDAV 密码无法读取：{}", path.display());
    };
    let mut secrets = BTreeMap::new();
    for (account, password) in object {
        let Some(password) = password.as_str() else {
            bail!("已保存的 WebDAV 密码无法读取：{}", path.display());
        };
        secrets.insert(account, password.to_owned());
    }
    Ok(secrets)
}

fn write(secrets: &BTreeMap<String, String>) -> Result<()> {
    let path = secrets_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法保存 WebDAV 密码：{}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(secrets).context("无法保存 WebDAV 密码")?;
    let temporary = path.with_extension("json.tmp");
    let write_temporary = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })()
    .with_context(|| format!("无法保存 WebDAV 密码：{}", temporary.display()));
    if let Err(error) = write_temporary {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    replace_file(&temporary, &path)
}

fn replace_file(temporary: &Path, path: &Path) -> Result<()> {
    if std::fs::rename(temporary, path).is_ok() {
        return Ok(());
    }
    if cfg!(windows) {
        let _ = std::fs::remove_file(path);
        if std::fs::rename(temporary, path).is_ok() {
            return Ok(());
        }
    }
    let error = std::io::Error::other(format!("无法替换 {}", path.display()));
    let _ = std::fs::remove_file(temporary);
    Err(error).with_context(|| format!("无法保存 WebDAV 密码：{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(url: &str, username: &str) -> Settings {
        Settings {
            url: url.into(),
            username: username.into(),
            auto: true,
        }
    }

    #[test]
    fn password_round_trip_is_scoped_to_vault_and_account() {
        let root =
            std::env::temp_dir().join(format!("inkstone-webdav-secret-{}", std::process::id()));
        let other = root.join("other");
        let saved = settings("https://example.test/dav", "alice");
        let account = account(&saved, &root).unwrap();
        store(&account, "session-secret").unwrap();
        assert_eq!(
            load(&saved, &root).unwrap().as_deref(),
            Some("session-secret")
        );
        assert!(load(&saved, &other).unwrap().is_none());
        assert!(
            load(&settings("https://example.test/dav/", "bob"), &root)
                .unwrap()
                .is_none()
        );
        let stored = std::fs::read_to_string(secrets_path()).unwrap();
        assert!(stored.contains("session-secret"));
        assert!(!stored.contains("https://example.test"));
        assert!(!stored.contains("alice"));
        assert!(!root.join(FILE_NAME).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(secrets_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store(&account, "").unwrap();
        assert!(load(&saved, &root).unwrap().is_none());
        store(&account, "").unwrap();
    }
}
