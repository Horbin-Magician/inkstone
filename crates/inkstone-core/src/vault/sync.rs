//! Versioned, content-addressed WebDAV sync. Only the manifest is mutable remotely.
//! Local baselines are device-owned; credentials never enter a manifest or baseline.
pub mod recovery;
mod webdav;
use super::*;
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};
pub use webdav::WebDav;

pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
type Files = BTreeMap<String, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub url: String,
    pub username: String,
    /// Sync after opening a vault and after local files are created, saved, or deleted.
    /// Absent in older preference files, so those vaults keep syncing automatically.
    pub auto: bool,
    /// Low-frequency remote checks while the application remains open.
    pub poll_minutes: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            auto: true,
            poll_minutes: 5,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        self.canonical_url().map(|_| ())
    }
    /// Normalized directory URL. Passwords are never part of this value.
    pub fn canonical_url(&self) -> Result<String> {
        Ok(webdav::parse_url(self)?.to_string())
    }
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
pub trait Remote: Sync {
    fn manifest(&self) -> Result<(Manifest, Option<String>)>;
    fn download(&self, hash: &str) -> Result<Vec<u8>>;
    /// Receive an object into a caller-owned staging sink. Production transports
    /// should override this compatibility fallback to avoid buffering the object.
    fn download_to(&self, hash: &str, output: &mut dyn Write) -> Result<u64> {
        let bytes = self.download(hash)?;
        ensure!(bytes.len() as u64 <= MAX_FILE_BYTES, "云端文件超过大小限制");
        output.write_all(&bytes)?;
        Ok(bytes.len() as u64)
    }
    fn upload(&self, hash: &str, bytes: &[u8]) -> Result<()>;
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Scanning,
    ReadingManifest,
    Downloading,
    Uploading,
    Verifying,
    Publishing,
    Applying,
    Complete,
}

/// Counts are per phase. Transfer counts refer to unique content objects;
/// bytes count completed objects, not partial network writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub completed: usize,
    pub total: usize,
    pub bytes: u64,
}
impl Progress {
    fn new(phase: Phase, total: usize) -> Self {
        Self {
            phase,
            completed: 0,
            total,
            bytes: 0,
        }
    }
}

#[derive(Default, Debug)]
pub struct Report {
    /// Unix milliseconds, recorded only when the local baseline commits.
    pub completed_at_ms: u64,
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
                && part.len() <= 255
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
#[cfg(test)]
fn snapshot(vault: &Vault) -> Result<Files> {
    snapshot_with_progress(vault, Phase::Scanning, &|_| {})
}
fn snapshot_with_progress(
    vault: &Vault,
    phase: Phase,
    notify: &impl Fn(Progress),
) -> Result<Files> {
    snapshot_cancellable(vault, phase, notify, &Cancellation::default())
}
fn snapshot_cancellable(
    vault: &Vault,
    phase: Phase,
    notify: &impl Fn(Progress),
    cancellation: &Cancellation,
) -> Result<Files> {
    notify(Progress::new(phase, 0));
    cancellation.check()?;
    let paths: Vec<_> = vault
        .scan_files()?
        .into_iter()
        .filter(|path| {
            !path
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
        .collect();
    let mut progress = Progress::new(phase, paths.len());
    notify(progress.clone());
    let mut files = Files::new();
    for path in paths {
        cancellation.check()?;
        let name = path
            .to_str()
            .context("同步文件名必须是 UTF-8")?
            .replace('\\', "/");
        validate_path(&name)?;
        let bytes =
            read_file(&vault.regular_file_path(&path)?)?.context("扫描期间文件被删除，请重试")?;
        files.insert(name, hash(&bytes));
        progress.completed += 1;
        progress.bytes += bytes.len() as u64;
        notify(progress.clone());
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

// Bound network concurrency and stop claiming work after the first failure. All
// in-flight requests finish before the caller can publish or return an error.
fn transfer<I: Sync>(items: &[I], work: impl Fn(&I) -> Result<()> + Sync) -> Result<()> {
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let error = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..items.len().min(4) {
            scope.spawn(|| {
                while !stopped.load(Ordering::Acquire) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    if let Err(e) = work(item) {
                        stopped.store(true, Ordering::Release);
                        error.lock().unwrap().get_or_insert(e);
                        break;
                    }
                }
            });
        }
    });
    match error.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Blocking; run on a worker. `identity` identifies the endpoint/account, not its password.
fn baseline_path(vault: &Vault, identity: &str) -> PathBuf {
    vault.recovery_dir.join("webdav-sync").join(format!(
        "{}-{}.json",
        hash(vault.root.to_string_lossy().as_bytes()),
        hash(identity.as_bytes())
    ))
}

#[derive(Serialize, Deserialize)]
struct LocalBaseline {
    #[serde(flatten)]
    manifest: Manifest,
    #[serde(default)]
    completed_at_ms: Option<u64>,
}

/// Last fully committed synchronization for this vault/account. Run on a worker.
/// Older records deliberately return None rather than guessing from file mtime.
pub fn last_success(vault: &Vault, identity: &str) -> Result<Option<u64>> {
    let bytes = match fs::read(baseline_path(vault, identity)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let record: LocalBaseline =
        serde_json::from_slice(&bytes).context("本地同步记录损坏，请保留记录并检查")?;
    validate_manifest(&record.manifest)?;
    Ok(record.completed_at_ms)
}

fn read_baseline(path: &Path) -> Result<Manifest> {
    let base: Manifest = match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).context("本地同步记录损坏，请保留记录并检查")?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Manifest::default(),
        Err(e) => return Err(e.into()),
    };
    validate_manifest(&base)?;
    Ok(base)
}

/// Classify coalesced watcher events against the last completed sync. Run on a
/// worker after active synchronization finishes, so its own writes match the
/// committed baseline. Errors must remain visible/retryable, not mean unchanged.
/// Empty paths mean an uncertain/root event and require a full reconciliation.
pub fn changed_since_sync(
    vault: &Vault,
    identity: &str,
    paths: &BTreeSet<PathBuf>,
) -> Result<bool> {
    let base = read_baseline(&baseline_path(vault, identity))?;
    for relative in paths {
        if relative
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            // Reject traversal even though ordinary hidden files are excluded.
            ensure!(
                !relative
                    .components()
                    .any(|c| matches!(c, Component::ParentDir)),
                "同步路径无效"
            );
            continue;
        }
        if relative.as_os_str().is_empty() {
            return Ok(snapshot_with_progress(vault, Phase::Scanning, &|_| {})? != base.files);
        }
        let path = vault.regular_file_path(relative)?;
        let name = relative
            .to_str()
            .context("同步路径不是 UTF-8")?
            .replace('\\', "/");
        validate_path(&name)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => Some(metadata),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let prefix = format!("{name}/");
        if metadata.as_ref().is_some_and(|m| m.is_dir())
            || base.files.keys().any(|p| p.starts_with(&prefix))
        {
            // Directory creation/removal/rename may represent a whole subtree.
            return Ok(snapshot_with_progress(vault, Phase::Scanning, &|_| {})? != base.files);
        }
        let digest = if metadata.is_some() {
            let mut file = fs::File::open(path)?;
            ensure!(file.metadata()?.is_file(), "同步目标不是普通文件");
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut bytes = 0u64;
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
                ensure!(bytes <= MAX_FILE_BYTES, "单个同步文件超过 128 MiB");
                hasher.update(&buffer[..read]);
            }
            Some(format!("{:x}", hasher.finalize()))
        } else {
            None
        };
        if digest.as_ref() != base.files.get(&name) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One token per attempt. Cancellation is cooperative before publication; once
/// commit begins, publication, local apply and baseline commit finish together.
#[derive(Default)]
pub struct Cancellation(AtomicU8);
impl Cancellation {
    pub fn request(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn is_requested(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
    fn check(&self) -> Result<()> {
        if self.is_requested() {
            return Err(Cancelled.into());
        }
        Ok(())
    }
    fn begin_commit(&self) -> Result<()> {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Cancelled.into())
    }
}
#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("同步已取消；未发布清单或修改本地文件")
    }
}
impl std::error::Error for Cancelled {}
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.is::<Cancelled>()
}

pub fn synchronize(vault: &Vault, remote: &impl Remote, identity: &str) -> Result<Report> {
    synchronize_with_progress(vault, remote, identity, |_| {})
}

/// `notify` runs on worker threads; callbacks are serialized within each phase.
pub fn synchronize_with_progress(
    vault: &Vault,
    remote: &impl Remote,
    identity: &str,
    notify: impl Fn(Progress) + Sync,
) -> Result<Report> {
    synchronize_cancellable(vault, remote, identity, &Cancellation::default(), notify)
}

/// In-flight network calls finish before cancellation returns; no new transfers
/// are started after observing cancellation. Use a fresh token for each attempt.
pub fn synchronize_cancellable(
    vault: &Vault,
    remote: &impl Remote,
    identity: &str,
    cancellation: &Cancellation,
    notify: impl Fn(Progress) + Sync,
) -> Result<Report> {
    cancellation.check()?;
    let device = vault.recovery_dir.join("webdav-sync");
    fs::create_dir_all(&device)?;
    let vault_id = hash(vault.root.to_string_lossy().as_bytes());
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(device.join(format!("{vault_id}.lock")))?;
    lock.try_lock().context("该笔记库已有同步任务运行")?;
    let state = baseline_path(vault, identity);
    let base = read_baseline(&state)?;
    let local = snapshot_cancellable(vault, Phase::Scanning, &notify, cancellation)?;
    notify(Progress::new(Phase::ReadingManifest, 0));
    cancellation.check()?;
    let (previous, revision) = remote.manifest()?;
    cancellation.check()?;
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
    let needed: Vec<_> = manifest
        .files
        .iter()
        .filter(|(path, digest)| local.get(*path) != Some(*digest))
        .collect();
    let digests: Vec<_> = needed
        .iter()
        .map(|(_, digest)| *digest)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let downloaded = Mutex::new((0usize, BTreeMap::new()));
    notify(Progress::new(Phase::Downloading, digests.len()));
    // Fetch each content object only once, even when several paths use it.
    transfer(&digests, |digest| {
        cancellation.check()?;
        let bytes = remote.download(digest)?;
        cancellation.check()?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES && hash(&bytes) == **digest,
            "云端文件校验失败：{digest}"
        );
        let mut downloaded = downloaded.lock().unwrap();
        ensure!(
            downloaded.0 + bytes.len() <= MAX_TOTAL_BYTES,
            "本次下载超过 512 MiB，当前版本暂不支持"
        );
        downloaded.0 += bytes.len();
        downloaded.1.insert((*digest).clone(), Arc::new(bytes));
        notify(Progress {
            phase: Phase::Downloading,
            completed: downloaded.1.len(),
            total: digests.len(),
            bytes: downloaded.0 as u64,
        });
        Ok(())
    })?;
    let (_, objects) = downloaded.into_inner().unwrap();
    let mut downloads = BTreeMap::new();
    let mut total = 0;
    // Validate every target before publishing or changing local files.
    for (path, digest) in needed {
        cancellation.check()?;
        let bytes = objects.get(digest).unwrap().clone();
        total += bytes.len();
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "本次下载超过 512 MiB，当前版本暂不支持"
        );
        let target = vault.regular_file_path(Path::new(path))?;
        ensure!(
            read_file(&target)?.as_ref().map(|b| hash(b)).as_ref() == local.get(path),
            "本地文件已变化：{path}"
        );
        downloads.insert(path.clone(), bytes);
    }
    let known: BTreeSet<_> = previous.files.values().collect();
    let mut uploaded = BTreeSet::new();
    let uploads: Vec<_> = manifest
        .files
        .iter()
        .filter(|(_, digest)| !known.contains(*digest) && uploaded.insert(*digest))
        .collect();
    let upload_progress = Mutex::new(Progress::new(Phase::Uploading, uploads.len()));
    notify(upload_progress.lock().unwrap().clone());
    transfer(&uploads, |(path, digest)| {
        cancellation.check()?;
        let bytes =
            read_file(&vault.regular_file_path(Path::new(path))?)?.context("上传期间文件被删除")?;
        ensure!(hash(&bytes) == **digest, "上传期间文件已变化：{path}");
        cancellation.check()?;
        remote.upload(digest, &bytes)?;
        cancellation.check()?;
        let mut progress = upload_progress.lock().unwrap();
        progress.completed += 1;
        progress.bytes += bytes.len() as u64;
        notify(progress.clone());
        Ok(())
    })?;
    report.uploaded = uploads.len();
    ensure!(
        snapshot_cancellable(vault, Phase::Verifying, &notify, cancellation)? == local,
        "同步期间本地文件已变化，请重试"
    );
    notify(Progress::new(Phase::Publishing, 0));
    cancellation.begin_commit()?;
    if manifest.files != previous.files || revision.is_none() {
        remote.publish(&manifest, revision.as_deref())?;
    }
    // A failed/ambiguous publication never applies downloads or advances the baseline.
    let deletions = local
        .keys()
        .filter(|path| !manifest.files.contains_key(*path))
        .count();
    let mut progress = Progress::new(Phase::Applying, downloads.len() + deletions);
    notify(progress.clone());
    for (path, bytes) in downloads {
        apply(vault, &path, local.get(&path), Some(&bytes))?;
        report.downloaded += 1;
        progress.completed += 1;
        notify(progress.clone());
    }
    for (path, digest) in &local {
        if !manifest.files.contains_key(path) {
            apply(vault, path, Some(digest), None)?;
            report.deleted += 1;
            progress.completed += 1;
            notify(progress.clone());
        }
    }
    let temp = state.with_extension("pending");
    let completed_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    let record = LocalBaseline {
        manifest,
        completed_at_ms: Some(completed_at_ms),
    };
    fs::write(&temp, serde_json::to_vec(&record)?)?;
    fs::File::open(&temp)?.sync_all()?;
    fs::rename(temp, state)?;
    report.completed_at_ms = completed_at_ms;
    notify(Progress::new(Phase::Complete, 0));
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
    if baseline.is_some() {
        // Keep recovery files self-describing even if the process stops mid-apply.
        write_new_synced(
            &backup.with_extension("backup.json"),
            &serde_json::to_vec(
                &serde_json::json!({"original": relative, "backup": backup.file_name().and_then(|n| n.to_str()), "sha256": baseline}),
            )?,
        )?;
    }
    if let Some(bytes) = bytes {
        let temp = parent.join(format!(".inkstone-sync-{}.tmp", unique_id()));
        write_new_synced(&temp, bytes)?;
        let result = (|| -> Result<()> {
            #[cfg(unix)]
            if baseline.is_some() {
                fs::set_permissions(&temp, fs::metadata(&path)?.permissions())?;
                fs::File::open(&temp)?.sync_all()?;
            }
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
