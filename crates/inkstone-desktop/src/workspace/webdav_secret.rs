//! WebDAV passwords stay in the OS credential store, never in the vault.
use anyhow::{Context, Result, bail};
use inkstone_core::vault::sync::Settings;
use keyring::{Entry, Error};
use sha2::{Digest, Sha256};
use std::path::Path;

const SERVICE: &str = "Inkstone WebDAV";

pub(super) fn supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

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
    if !supported() || settings.username.trim().is_empty() || settings.url.trim().is_empty() {
        return Ok(None);
    }
    let account = account(settings, vault_root)?;
    read(&account)
}

pub(super) fn store(account: &str, password: &str) -> Result<()> {
    if password.is_empty() {
        return forget(account);
    }
    write(account, password)
}

fn entry(account: &str) -> Result<Entry> {
    Entry::new(SERVICE, account).context("无法打开系统钥匙串")
}

fn read(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(store_error("无法读取已保存的 WebDAV 密码", error)),
    }
}

fn write(account: &str, password: &str) -> Result<()> {
    entry(account)?
        .set_password(password)
        .map_err(|error| store_error("无法保存 WebDAV 密码", error))
}

fn forget(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(store_error("无法清除已保存的 WebDAV 密码", error)),
    }
}

fn store_error(action: &str, error: Error) -> anyhow::Error {
    let detail = match error {
        Error::NoStorageAccess(_) => "系统钥匙串当前不可用，请解锁后重试".to_owned(),
        Error::Ambiguous(_) => "系统钥匙串中有多项匹配，请清理后重试".to_owned(),
        Error::TooLong(_, _) => "密码超过系统钥匙串长度限制".to_owned(),
        Error::BadEncoding(_) => "已保存的密码无法读取".to_owned(),
        other => other.to_string(),
    };
    anyhow::anyhow!("{action}：{detail}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use keyring::credential::{
        Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence,
    };
    use std::collections::HashMap;
    use std::sync::{Mutex, Once, OnceLock};

    fn memory() -> &'static Mutex<HashMap<String, String>> {
        static MEMORY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        MEMORY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    #[derive(Debug)]
    struct MemoryCredential {
        key: String,
    }
    impl CredentialApi for MemoryCredential {
        fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
            let password = String::from_utf8(secret.to_vec())
                .map_err(|_| keyring::Error::BadEncoding(secret.to_vec()))?;
            memory().lock().unwrap().insert(self.key.clone(), password);
            Ok(())
        }
        fn get_secret(&self) -> keyring::Result<Vec<u8>> {
            memory()
                .lock()
                .unwrap()
                .get(&self.key)
                .map(|password| password.as_bytes().to_vec())
                .ok_or(keyring::Error::NoEntry)
        }
        fn delete_credential(&self) -> keyring::Result<()> {
            memory()
                .lock()
                .unwrap()
                .remove(&self.key)
                .map(|_| ())
                .ok_or(keyring::Error::NoEntry)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[derive(Debug)]
    struct MemoryBuilder;
    impl CredentialBuilderApi for MemoryBuilder {
        fn build(
            &self,
            _: Option<&str>,
            service: &str,
            user: &str,
        ) -> keyring::Result<Box<Credential>> {
            Ok(Box::new(MemoryCredential {
                key: format!("{service}\n{user}"),
            }))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn persistence(&self) -> CredentialPersistence {
            CredentialPersistence::ProcessOnly
        }
    }

    pub(crate) fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            keyring::set_default_credential_builder(Box::new(MemoryBuilder));
        });
    }

    fn settings(url: &str, username: &str) -> Settings {
        Settings {
            url: url.into(),
            username: username.into(),
        }
    }

    #[test]
    fn password_round_trip_is_scoped_to_vault_and_account() {
        install();
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
        store(&account, "").unwrap();
        assert!(load(&saved, &root).unwrap().is_none());
        store(&account, "").unwrap();
    }
}
