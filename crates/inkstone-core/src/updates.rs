//! Release selection and bounded, cancellable downloads. SHA-256 checks integrity;
//! trust comes from HTTPS and the project's GitHub release, not a signing key.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const REPOSITORY: &str = "https://github.com/Horbin-Magician/inkstone";
const ENDPOINT: &str = "https://api.github.com/repos/Horbin-Magician/inkstone/releases/latest";
const MAX_METADATA: u64 = 1024 * 1024;
const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    name: String,
    url: String,
    checksum_url: String,
    size: u64,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn select_release(bytes: &[u8], current: &str, target: &str) -> Result<Option<Release>> {
    let release: GithubRelease = serde_json::from_slice(bytes).context("版本信息格式无效")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("发布版本号无效")?;
    let current = semver::Version::parse(current)?;
    if !version.pre.is_empty() || version.cmp_precedence(&current).is_le() {
        return Ok(None);
    }
    let suffix = match target {
        "aarch64-apple-darwin" => ".dmg",
        "x86_64-pc-windows-msvc" => "-setup.exe",
        _ => bail!("暂不支持当前系统的应用内更新"),
    };
    let stem = format!("inkstone-{version}-{target}");
    let name = format!("{stem}{suffix}");
    let checksum_name = format!("{stem}.sha256");
    let find = |name: &str| -> Result<&Asset> {
        let mut matches = release.assets.iter().filter(|asset| asset.name == name);
        let asset = matches.next().context("此版本缺少安装包或校验文件")?;
        ensure!(matches.next().is_none(), "发布附件名称重复");
        let expected = format!("{REPOSITORY}/releases/download/{}/{name}", release.tag_name);
        ensure!(
            asset.browser_download_url == expected,
            "发布附件地址不属于此项目或版本"
        );
        Ok(asset)
    };
    let artifact = find(&name)?;
    let checksums = find(&checksum_name)?;
    ensure!(
        (1..=MAX_DOWNLOAD).contains(&artifact.size),
        "安装包大小无效"
    );
    ensure!(
        (1..=MAX_METADATA).contains(&checksums.size),
        "校验文件大小无效"
    );
    Ok(Some(Release {
        version: version.to_string(),
        notes: release.body.clone().unwrap_or_default(),
        page: format!("{REPOSITORY}/releases/tag/{}", release.tag_name),
        name,
        url: artifact.browser_download_url.clone(),
        checksum_url: checksums.browser_download_url.clone(),
        size: artifact.size,
    }))
}

fn client(timeout: u64) -> Result<Client> {
    Client::builder()
        .https_only(true)
        .user_agent(concat!("Inkstone/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(timeout))
        .build()
        .context("无法初始化更新连接")
}

fn bounded_read(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "更新信息超过大小限制");
    Ok(bytes)
}

pub fn check(current: &str) -> Result<Option<Release>> {
    let target = target().context("暂不支持当前系统的应用内更新")?;
    let response = client(30)?
        .get(ENDPOINT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("连接 GitHub 失败，请稍后重试")?;
    // No published releases is distinct from an offline/failed check.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("尚未找到公开发行版本");
    }
    let response = response
        .error_for_status()
        .context("获取版本失败（可能触发 GitHub 请求限制），请稍后重试")?;
    select_release(&bounded_read(response, MAX_METADATA)?, current, target)
}

fn checksum(bytes: &[u8], name: &str) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("校验文件编码无效")?;
    let mut found = None;
    for line in text.lines() {
        let Some((hash, file)) = line.split_once("  ") else {
            bail!("校验文件格式无效")
        };
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "SHA-256 格式无效"
        );
        if file == name {
            ensure!(found.is_none(), "校验文件包含重复条目");
            found = Some(hash.to_ascii_lowercase());
        }
    }
    found.context("校验文件中缺少安装包条目")
}

/// Own the private download directory for as long as its installer is offered.
/// Dropping this value removes the downloaded package and any partial data.
#[derive(Debug)]
pub struct Download {
    directory: tempfile::TempDir,
    pub path: PathBuf,
    hash: String,
    size: u64,
}
impl Download {
    /// Recheck on open, so a stale or modified cached file cannot be launched.
    pub fn verify(&self) -> Result<()> {
        let file = File::open(&self.path)?;
        ensure!(
            file.metadata()?.len() == self.size,
            "安装包大小已改变，请重新下载"
        );
        let mut hasher = Sha256::new();
        let size = std::io::copy(&mut file.take(MAX_DOWNLOAD + 1), &mut hasher)?;
        ensure!(
            size == self.size && format!("{:x}", hasher.finalize()) == self.hash,
            "安装包校验失败，请重新下载"
        );
        Ok(())
    }

    /// Keep packages until the installer/OS has finished consuming them. The UI
    /// retains this object after handing off; this accessor is for revealing it.
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

pub fn download(
    release: &Release,
    cache: &Path,
    cancelled: &AtomicBool,
    progress: impl Fn(u64, u64),
) -> Result<Download> {
    ensure!(!cancelled.load(Ordering::Relaxed), "下载已取消");
    let client = client(600)?;
    let hashes = client
        .get(&release.checksum_url)
        .timeout(Duration::from_secs(30))
        .send()?
        .error_for_status()?;
    let hash = checksum(&bounded_read(hashes, MAX_METADATA)?, &release.name)?;
    ensure!(!cancelled.load(Ordering::Relaxed), "下载已取消");
    std::fs::create_dir_all(cache)?;
    let directory = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(cache)?;
    let path = directory.path().join(&release.name);
    let mut response = client.get(&release.url).send()?.error_for_status()?;
    if let Some(length) = response.content_length() {
        ensure!(length == release.size, "下载大小与发布信息不一致");
    }
    let mut output = File::create(&path)?;
    copy_verified(
        &mut response,
        &mut output,
        release.size,
        &hash,
        cancelled,
        progress,
    )?;
    output.sync_all()?;
    drop(output);
    Ok(Download {
        directory,
        path,
        hash,
        size: release.size,
    })
}

fn copy_verified(
    input: &mut impl Read,
    output: &mut impl Write,
    expected: u64,
    hash: &str,
    cancelled: &AtomicBool,
    progress: impl Fn(u64, u64),
) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut downloaded = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        ensure!(!cancelled.load(Ordering::Relaxed), "下载已取消");
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        downloaded += count as u64;
        ensure!(
            downloaded <= expected && downloaded <= MAX_DOWNLOAD,
            "下载超过大小限制"
        );
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        progress(downloaded, expected);
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "下载已取消");
    ensure!(downloaded == expected, "安装包下载不完整");
    ensure!(
        format!("{:x}", hasher.finalize()) == hash,
        "安装包 SHA-256 校验失败，请重新下载"
    );
    Ok(())
}

/// Launch only a verified package, using an argument (never a shell command).
/// Successful handoff preserves the package for the external installer.
pub fn open_installer(download: Download) -> Result<PathBuf> {
    #[cfg(windows)]
    let _guard = {
        use std::os::windows::fs::OpenOptionsExt;
        File::options()
            .read(true)
            .share_mode(1)
            .open(&download.path)?
    };
    download.verify()?;
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(&download.path)
            .status()?;
        ensure!(status.success(), "无法打开安装包");
    }
    #[cfg(windows)]
    std::process::Command::new(&download.path)
        .spawn()
        .context("无法启动安装程序")?;
    #[cfg(not(any(target_os = "macos", windows)))]
    bail!("暂不支持当前系统的安装包");
    #[cfg(any(target_os = "macos", windows))]
    {
        let path = download.path.clone();
        let _ = download.directory.keep();
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture(version: &str, target: &str, suffix: &str) -> Value {
        let stem = format!("inkstone-{version}-{target}");
        json!({"tag_name": format!("v{version}"), "body": "Release notes", "draft": false,
            "prerelease": false, "assets": ([format!("{stem}{suffix}"), format!("{stem}.sha256")]
                .map(|name| json!({"browser_download_url": format!("{REPOSITORY}/releases/download/v{version}/{name}"), "name": name, "size": 100})))})
    }
    fn select(value: &Value, current: &str, target: &str) -> Result<Option<Release>> {
        select_release(&serde_json::to_vec(value).unwrap(), current, target)
    }
    #[test]
    fn semantic_versions_and_platform_packages() {
        for (target, suffix) in [
            ("aarch64-apple-darwin", ".dmg"),
            ("x86_64-pc-windows-msvc", "-setup.exe"),
        ] {
            let value = fixture("0.10.0", target, suffix);
            let release = select(&value, "0.9.0", target).unwrap().unwrap();
            assert!(release.name.ends_with(suffix));
            assert_eq!(release.notes, "Release notes");
            for current in ["0.10.0", "0.11.0", "0.10.0+local"] {
                assert!(select(&value, current, target).unwrap().is_none());
            }
            assert!(select(&value, "0.10.0-beta.1", target).unwrap().is_some());
        }
    }
    #[test]
    fn reject_incomplete_untrusted_and_invalid_releases() {
        let target = "aarch64-apple-darwin";
        let value = fixture("1.0.0", target, ".dmg");
        for field in ["draft", "prerelease"] {
            let mut value = value.clone();
            value[field] = json!(true);
            assert!(select(&value, "0.1.0", target).unwrap().is_none());
        }
        assert!(
            select(&fixture("1.0.0-beta.1", target, ".dmg"), "0.1.0", target)
                .unwrap()
                .is_none()
        );
        for url in [
            "http://github.com/file",
            "https://evil.test/file",
            "https://github.com/Horbin-Magician/rotor/releases/download/v1.0.0/file",
        ] {
            let mut value = value.clone();
            value["assets"][0]["browser_download_url"] = json!(url);
            assert!(select(&value, "0.1.0", target).is_err());
        }
        let mut missing = value.clone();
        missing["assets"].as_array_mut().unwrap().pop();
        assert!(select(&missing, "0.1.0", target).is_err());
        let mut duplicate = value.clone();
        duplicate["assets"]
            .as_array_mut()
            .unwrap()
            .push(value["assets"][0].clone());
        assert!(select(&duplicate, "0.1.0", target).is_err());
        let mut large = value.clone();
        large["assets"][0]["size"] = json!(MAX_DOWNLOAD + 1);
        assert!(select(&large, "0.1.0", target).is_err());
        assert!(select(&value, "0.1.0", "unknown").is_err());
    }
    #[test]
    fn checksum_requires_exact_unique_filename_and_valid_hash() {
        let hash = "ab".repeat(32);
        assert_eq!(
            checksum(format!("{hash}  a.dmg\n").as_bytes(), "a.dmg").unwrap(),
            hash
        );
        for text in [
            format!("{hash}  b.dmg"),
            format!("{hash}  a.dmg\n{hash}  a.dmg"),
            "zz  a.dmg".into(),
        ] {
            assert!(checksum(text.as_bytes(), "a.dmg").is_err());
        }
        assert!(bounded_read(&b"12345"[..], 4).is_err());
    }
    #[test]
    fn stream_checks_hash_length_and_cancellation() {
        let bytes = b"verified installer";
        let hash = format!("{:x}", Sha256::digest(bytes));
        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        copy_verified(
            &mut &bytes[..],
            &mut output,
            bytes.len() as u64,
            &hash,
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(output, bytes);
        for (size, expected_hash) in [
            (1, hash.as_str()),
            (100, hash.as_str()),
            (bytes.len() as u64, "wrong"),
        ] {
            assert!(
                copy_verified(
                    &mut &bytes[..],
                    &mut Vec::new(),
                    size,
                    expected_hash,
                    &cancel,
                    |_, _| {}
                )
                .is_err()
            );
        }
        assert!(
            copy_verified(
                &mut &bytes[..],
                &mut Vec::new(),
                bytes.len() as u64,
                &hash,
                &cancel,
                |_, _| cancel.store(true, Ordering::Relaxed)
            )
            .is_err()
        );
        assert!(
            copy_verified(
                &mut &bytes[..],
                &mut Vec::new(),
                bytes.len() as u64,
                &hash,
                &cancel,
                |_, _| {}
            )
            .is_err()
        );
    }
    #[test]
    fn cached_file_is_reverified_and_removed_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("installer.dmg");
        std::fs::write(&path, b"original").unwrap();
        let download = Download {
            directory,
            path: path.clone(),
            hash: format!("{:x}", Sha256::digest(b"original")),
            size: 8,
        };
        download.verify().unwrap();
        std::fs::write(&path, b"tampered").unwrap();
        assert!(download.verify().is_err());
        drop(download);
        assert!(!path.exists());
    }
}
