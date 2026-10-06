//! Locked, full-integrity preflight for an explicitly reviewed retention preview.
use super::{
    retention::{Decision, Preview},
    *,
};

/// A checked plan retains the directory lock until dropped. This is not a
/// persistent authorization: external file edits still require rechecking at use.
pub struct Checked {
    preview: Preview,
    _lock: crate::vault::file_lock::FileLock,
    directory: PathBuf,
    in_use: BTreeSet<PathBuf>,
    snapshots: Vec<(Manifest, Option<String>)>,
}
impl Checked {
    pub fn preview(&self) -> &Preview {
        &self.preview
    }
}

pub fn prepare(
    directory: &Path,
    expected: &Preview,
    in_use: &BTreeSet<PathBuf>,
) -> io::Result<Checked> {
    // Keep the path spelling used by the reviewed inventory; the lock helper
    // canonicalizes aliases for coordination. Fresh listing rejects root links.
    let directory = directory.to_owned();
    super::storage::require_local(&directory)?;
    let lock = locking::acquire(&directory, true)?;
    let snapshots = validate(&directory, expected, in_use)?;
    Ok(Checked {
        preview: expected.clone(),
        _lock: lock,
        directory,
        in_use: in_use.clone(),
        snapshots,
    })
}

fn validate(
    directory: &Path,
    expected: &Preview,
    in_use: &BTreeSet<PathBuf>,
) -> io::Result<Vec<(Manifest, Option<String>)>> {
    let inventory = capacity::list(directory)?;
    if !inventory.interrupted.is_empty() {
        return Err(invalid(
            "存在未处理的清理隔离记录，请先检查并恢复需要的内容",
        ));
    }
    if inventory.unreadable != 0 {
        return Err(invalid("备份清单存在异常，请先检查异常记录"));
    }
    let fresh = retention::preview(&inventory, expected.keep_per_source, in_use)?;
    if &fresh != expected {
        return Err(invalid("备份或保护状态已变化，请重新预览"));
    }
    if fresh.candidates == 0 {
        return Err(invalid("没有可清理的备份候选"));
    }
    let mut snapshots = Vec::new();
    for record in &fresh.records {
        let path = &record.backup.directory;
        if path.parent() != Some(directory) {
            return Err(invalid("备份不在选定位置内"));
        }
        // Retained backups must be usable too: a same-length corrupt newest
        // backup must never justify deleting the last good older copy.
        let manifest = inspect_unlocked(path)?;
        let identity = origin::id(path)?;
        if record.decision == Decision::Candidate && identity.is_none() {
            return Err(invalid("无法确认候选目录身份，已保留备份"));
        }
        snapshots.push((manifest, identity));
        if record.decision == Decision::Candidate {
            for entry in fs::read_dir(path)? {
                let name = entry?.file_name();
                if name != "files" && name != "manifest.json" {
                    return Err(invalid("清理候选包含额外文件，请人工检查"));
                }
            }
        }
    }
    // Catch metadata changes during the integrity scan as well as before it.
    if capacity::list(directory)? != inventory {
        return Err(invalid("校验期间备份发生变化，请重新预览"));
    }
    Ok(snapshots)
}

#[derive(Debug, Default)]
pub struct Report {
    pub removed: usize,
    pub logical_bytes: u64,
}
impl Checked {
    /// Execute for local storage with all writers honoring the same lock protocol.
    /// This API does not coordinate other machines or non-cooperating writers.
    pub fn execute(self) -> io::Result<Report> {
        self.execute_with(|_, _| {})
    }
    pub(super) fn execute_with(
        self,
        mut after_move: impl FnMut(&Path, &Path),
    ) -> io::Result<Report> {
        super::storage::require_local(&self.directory)?;
        if validate(&self.directory, &self.preview, &self.in_use)? != self.snapshots {
            return Err(invalid("备份身份或清单在预检后发生变化"));
        }
        let mut report = Report::default();
        for (record, (manifest, identity)) in self.preview.records.iter().zip(&self.snapshots) {
            if record.decision != Decision::Candidate {
                continue;
            }
            let source = &record.backup.directory;
            let quarantine = self
                .directory
                .join(format!(".inkstone-backup-cleanup-{}", unique_id()));
            let result = (|| -> io::Result<()> {
                move_no_replace(source, &quarantine)?;
                let verified = (|| -> io::Result<()> {
                    after_move(source, &quarantine);
                    if origin::id_at(&quarantine, source)? != *identity
                        || inspect_unlocked(&quarantine)? != *manifest
                    {
                        return Err(invalid("清理候选在隔离期间发生变化"));
                    }
                    for entry in fs::read_dir(&quarantine)? {
                        let name = entry?.file_name();
                        if name != "files" && name != "manifest.json" {
                            return Err(invalid("候选出现额外文件"));
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = verified {
                    return match move_no_replace(&quarantine, source) {
                        Ok(()) => Err(error),
                        Err(_) => Err(invalid(format!(
                            "{error}；无法返回原位置，内容保留在 {}",
                            quarantine.display()
                        ))),
                    };
                }
                fs::remove_dir_all(&quarantine).map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("清理未完成：{error}；剩余内容位于 {}", quarantine.display()),
                    )
                })?;
                Ok(())
            })();
            if let Err(error) = result {
                return Err(io::Error::new(
                    error.kind(),
                    format!("已清理 {} 份；{error}", report.removed),
                ));
            }
            report.removed += 1;
            report.logical_bytes += record.backup.payload_bytes + record.backup.manifest_bytes;
        }
        Ok(report)
    }
}

const PROTECTION_FILE: &str = ".inkstone-backup-protected";
const PROTECTION: &[u8] = b"inkstone recovered backup protection v1\n";
pub(super) fn is_protected(directory: &Path) -> io::Result<bool> {
    let path = directory.join(PROTECTION_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || is_reparse(&metadata) || metadata.len() != PROTECTION.len() as u64 {
        return Err(invalid("备份保护标记无效"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(PROTECTION.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes != PROTECTION {
        return Err(invalid("备份保护标记无效"));
    }
    Ok(true)
}

/// Preserve a complete interrupted cleanup record as a protected backup under a
/// fresh name. Partial records stay untouched and must be inspected manually.
pub fn retain_interrupted(directory: &Path, selected: &Path) -> io::Result<PathBuf> {
    let _lock = locking::acquire(directory, true)?;
    if selected.parent() != Some(directory)
        || !selected.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with(".inkstone-backup-cleanup-")
        })
    {
        return Err(invalid("无效的清理中断记录"));
    }
    let manifest = inspect_unlocked(selected)?;
    let identity = origin::id(selected)?.ok_or_else(|| invalid("无法确认中断记录身份"))?;
    if !is_protected(selected)? {
        write_new_synced(&selected.join(PROTECTION_FILE), PROTECTION)?;
    }
    if origin::id(selected)?.as_ref() != Some(&identity) || inspect_unlocked(selected)? != manifest
    {
        return Err(invalid("保留期间备份发生变化，记录保持隔离"));
    }
    let destination = directory.join(format!("inkstone-backup-recovered-{}", unique_id()));
    move_no_replace(selected, &destination)?;
    Ok(destination)
}

/// This gate detects network/unknown filesystems, not cloud sync software or
/// non-cooperating writers on a local disk. Those still require caller exclusion.
pub fn supports_cleanup(directory: &Path) -> io::Result<bool> {
    super::storage::local(directory)
}
