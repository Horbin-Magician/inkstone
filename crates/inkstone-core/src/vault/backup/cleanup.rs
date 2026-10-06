//! Locked, full-integrity preflight for an explicitly reviewed retention preview.
use super::{
    retention::{Decision, Preview},
    *,
};

/// A checked plan retains the directory lock until dropped. This is not a
/// persistent authorization: external file edits still require rechecking at use.
pub struct Checked {
    preview: Preview,
    _lock: fs::File,
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
    let lock = locking::acquire(&directory, true)?;
    let inventory = capacity::list(&directory)?;
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
    for record in &fresh.records {
        let path = &record.backup.directory;
        if path.parent() != Some(directory.as_path()) {
            return Err(invalid("备份不在选定位置内"));
        }
        // Retained backups must be usable too: a same-length corrupt newest
        // backup must never justify deleting the last good older copy.
        inspect_unlocked(path)?;
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
    if capacity::list(&directory)? != inventory {
        return Err(invalid("校验期间备份发生变化，请重新预览"));
    }
    Ok(Checked {
        preview: fresh,
        _lock: lock,
    })
}
