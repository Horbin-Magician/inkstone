//! Versioned, content-addressed WebDAV sync. Only the manifest is mutable remotely.
//! Local baselines are device-owned; credentials never enter a manifest or baseline.
mod webdav;
use super::*;
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub use webdav::WebDav;

pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
type Files = BTreeMap<String, String>;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub url: String,
    pub username: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    version: u32,
    files: Files,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            files: Files::new(),
        }
    }
}

/// Implementations must atomically compare `revision` when publishing a manifest.
/// None means create only if absent. A stale revision must fail, never overwrite.
pub trait Remote {
    fn manifest(&self) -> Result<(Manifest, Option<String>)>;
    fn download(&self, hash: &str) -> Result<Vec<u8>>;
    fn upload(&self, hash: &str, bytes: &[u8]) -> Result<()>;
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()>;
}

#[derive(Default, Debug)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted: usize,
    pub conflicts: Vec<String>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty() && path.len() <= 4096, "同步路径无效");
    for part in path.split('/') {
        ensure!(
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with([' ', '.'])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "\\:<>\"|?*".contains(c)),
            "不支持的同步路径：{path}"
        );
        let stem = part.split('.').next().unwrap().to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "不支持的同步路径：{path}"
        );
    }
    Ok(())
}
fn validate_manifest(manifest: &Manifest) -> Result<()> {
    ensure!(manifest.version == 1, "不支持的云端同步格式");
    ensure!(manifest.files.len() <= 100_000, "同步文件数量超过限制");
    let mut names = BTreeSet::new();
    for (path, digest) in &manifest.files {
        validate_path(path)?;
        ensure!(valid_hash(digest), "无效的文件校验值");
        ensure!(
            names.insert(path.to_lowercase()),
            "存在大小写冲突的路径：{path}"
        );
    }
    for path in &names {
        let mut parent = Path::new(path).parent();
        while let Some(p) = parent {
            ensure!(
                !names.contains(p.to_str().unwrap()),
                "文件与目录路径冲突：{path}"
            );
            parent = p.parent();
        }
    }
    Ok(())
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(file.metadata()?.is_file(), "同步目标不是普通文件");
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "单个同步文件超过 128 MiB"
    );
    Ok(Some(bytes))
}
fn snapshot(vault: &Vault) -> Result<Files> {
    let mut files = Files::new();
    for path in vault.scan_files()? {
        if path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let name = path
            .to_str()
            .context("同步文件名必须是 UTF-8")?
            .replace('\\', "/");
        validate_path(&name)?;
        let bytes =
            read_file(&vault.regular_file_path(&path)?)?.context("扫描期间文件被删除，请重试")?;
        files.insert(name, hash(&bytes));
    }
    validate_manifest(&Manifest {
        version: 1,
        files: files.clone(),
    })?;
    Ok(files)
}
fn merge(base: &Files, local: &Files, remote: &Files) -> Result<(Files, Vec<String>)> {
    let mut merged = Files::new();
    let mut copies = Vec::new();
    let mut conflicts = Vec::new();
    for path in base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .collect::<BTreeSet<_>>()
    {
        let (b, l, r) = (base.get(path), local.get(path), remote.get(path));
        let chosen = if l == r {
            l
        } else if l == b {
            r
        } else if r == b {
            l
        } else {
            conflicts.push(path.clone());
            if let (Some(l), Some(r)) = (l, r) {
                // Keep the local version at the original name, remote beside it.
                let p = Path::new(path);
                let stem = p.file_stem().unwrap().to_str().unwrap();
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default();
                let name = p
                    .with_file_name(format!("{stem} (云端冲突 {r}){ext}"))
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                for files in [base, local, remote] {
                    ensure!(
                        files.get(&name).is_none_or(|h| h == r),
                        "冲突副本名称已被占用：{name}"
                    );
                }
                copies.push((name, r.clone()));
                Some(l)
            } else {
                l.or(r)
            } // Modification wins over concurrent deletion.
        };
        if let Some(value) = chosen {
            merged.insert(path.clone(), value.clone());
        }
    }
    merged.extend(copies);
    validate_manifest(&Manifest {
        version: 1,
        files: merged.clone(),
    })?;
    Ok((merged, conflicts))
}

/// Blocking; run on a worker. `identity` identifies the endpoint/account, not its password.
pub fn synchronize(vault: &Vault, remote: &impl Remote, identity: &str) -> Result<Report> {
    let device = vault.recovery_dir.join("webdav-sync");
    fs::create_dir_all(&device)?;
    let vault_id = hash(vault.root.to_string_lossy().as_bytes());
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(device.join(format!("{vault_id}.lock")))?;
    lock.try_lock().context("该笔记库已有同步任务运行")?;
    let state = device.join(format!("{}-{}.json", vault_id, hash(identity.as_bytes())));
    let base: Manifest = match fs::read(&state) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).context("本地同步记录损坏，请保留记录并检查")?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Manifest::default(),
        Err(e) => return Err(e.into()),
    };
    validate_manifest(&base)?;
    let local = snapshot(vault)?;
    let (previous, revision) = remote.manifest()?;
    validate_manifest(&previous)?;
    // Losing a previously populated remote is not interpreted as deleting the whole vault.
    ensure!(
        revision.is_some() || base.files.is_empty(),
        "云端清单缺失，已停止同步；请检查服务器目录"
    );
    let (files, conflicts) = merge(&base.files, &local, &previous.files)?;
    let manifest = Manifest { version: 1, files };
    let mut report = Report {
        conflicts,
        ..Default::default()
    };
    let mut downloads = BTreeMap::new();
    let mut total = 0;
    // Validate all downloads before publishing or changing local files.
    for (path, digest) in &manifest.files {
        if local.get(path) == Some(digest) {
            continue;
        }
        let bytes = remote.download(digest)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES && hash(&bytes) == *digest,
            "云端文件校验失败：{path}"
        );
        total += bytes.len();
        ensure!(total <= MAX_TOTAL_BYTES, "本次下载超过 512 MiB，请分批同步");
        // Reject symlinks, aliases and file/directory collisions before remote publication.
        let target = vault.regular_file_path(Path::new(path))?;
        ensure!(
            read_file(&target)?.as_ref().map(|b| hash(b)).as_ref() == local.get(path),
            "本地文件已变化：{path}"
        );
        downloads.insert(path.clone(), bytes);
    }
    let known: BTreeSet<_> = previous.files.values().collect();
    let mut uploaded = BTreeSet::new();
    for (path, digest) in &manifest.files {
        if known.contains(digest) || !uploaded.insert(digest) {
            continue;
        }
        let bytes =
            read_file(&vault.regular_file_path(Path::new(path))?)?.context("上传期间文件被删除")?;
        ensure!(hash(&bytes) == *digest, "上传期间文件已变化：{path}");
        remote.upload(digest, &bytes)?;
        report.uploaded += 1;
    }
    ensure!(snapshot(vault)? == local, "同步期间本地文件已变化，请重试");
    if manifest.files != previous.files || revision.is_none() {
        remote.publish(&manifest, revision.as_deref())?;
    }
    // A failed/ambiguous publication never applies downloads or advances the baseline.
    for (path, bytes) in downloads {
        apply(vault, &path, local.get(&path), Some(&bytes))?;
        report.downloaded += 1;
    }
    for (path, digest) in &local {
        if !manifest.files.contains_key(path) {
            apply(vault, path, Some(digest), None)?;
            report.deleted += 1;
        }
    }
    let temp = state.with_extension("pending");
    fs::write(&temp, serde_json::to_vec(&manifest)?)?;
    fs::File::open(&temp)?.sync_all()?;
    fs::rename(temp, state)?;
    Ok(report)
}

fn apply(
    vault: &Vault,
    relative: &str,
    baseline: Option<&String>,
    bytes: Option<&[u8]>,
) -> Result<()> {
    let path = vault.regular_file_path(Path::new(relative))?;
    ensure!(
        read_file(&path)?.as_ref().map(|b| hash(b)).as_ref() == baseline,
        "下载期间本地文件已变化：{relative}"
    );
    let parent = path.parent().context("无效的同步路径")?;
    fs::create_dir_all(parent)?;
    // Every displaced byte is retained in a hidden sibling. This also captures writes
    // in the compare/rename race; unlike remove_file, deletion never destroys content.
    let backup = parent.join(format!(".inkstone-sync-{}.backup", unique_id()));
    if let Some(bytes) = bytes {
        let temp = parent.join(format!(".inkstone-sync-{}.tmp", unique_id()));
        write_new_synced(&temp, bytes)?;
        let result = (|| -> Result<()> {
            if baseline.is_some() {
                replace_with_backup(&path, &temp, &backup)?;
            } else {
                fs::hard_link(&temp, &path)?;
            }
            Ok(())
        })();
        let _ = fs::remove_file(temp);
        result?;
    } else {
        move_no_replace(&path, &backup)?;
    }
    if baseline.is_some() && read_file(&backup)?.as_ref().map(|b| hash(b)).as_ref() != baseline {
        bail!(
            "同步时发生本地并发修改，原内容已保留在 {}",
            backup.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
