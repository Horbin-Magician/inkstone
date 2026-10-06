//! Files remain authoritative. Every save keeps an application-owned recovery journal.
pub mod attachments;
pub mod backup;
pub mod drafts;
pub mod export;
mod history;
pub mod sync;
mod trash_metadata;
pub use history::{HistoryEntry, Retention};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
pub use trash_metadata::{TrashMetadata, TrashRecord};

static SERIAL: AtomicU64 = AtomicU64::new(0);
fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug)]
pub enum VaultError {
    Io(io::Error),
    InvalidPath,
    InvalidUtf8,
    Conflict {
        recovery: PathBuf,
    },
    RaceConflict {
        recovery: PathBuf,
        external_backup: PathBuf,
    },
}
impl From<io::Error> for VaultError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "文件操作失败：{e}"),
            Self::InvalidPath => write!(
                f,
                "路径必须是笔记库内的普通 Markdown 文件，不能包含链接或上级目录"
            ),
            Self::InvalidUtf8 => write!(f, "文件不是有效 UTF-8，未修改内容"),
            Self::Conflict { recovery } => write!(
                f,
                "外部版本已改变，保存已停止。编辑副本：{}",
                recovery.display()
            ),
            Self::RaceConflict {
                recovery,
                external_backup,
            } => write!(
                f,
                "替换期间发生外部修改，两个版本均已保留。编辑副本：{}；外部版本：{}",
                recovery.display(),
                external_backup.display()
            ),
        }
    }
}
impl std::error::Error for VaultError {}

#[derive(Clone, Debug)]
pub struct Vault {
    pub root: PathBuf,
    pub recovery_dir: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    pub root: PathBuf,
    pub relative: PathBuf,
    pub baseline: Option<String>,
    pub draft: String,
}
#[derive(Debug)]
pub struct SaveReceipt {
    pub text: String,
    pub recovery: PathBuf,
}

/// Lightweight recovery list entry. Read and validate its body only when selected.
#[derive(Clone, Debug)]
pub struct RecoverySummary {
    pub journal: PathBuf,
    pub relative: PathBuf,
    pub modified: SystemTime,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct RecoveryEntry {
    pub journal: PathBuf,
    pub record: Recovery,
    pub modified: SystemTime,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct VaultTree {
    pub files: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct TrashEntry {
    pub stored: PathBuf,
    pub original: PathBuf,
    pub directory: bool,
}

fn is_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn read_optional(path: &Path) -> Result<Option<String>, VaultError> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| VaultError::InvalidUtf8),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn write_new_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

impl Vault {
    /// Duplicate the supplied editor snapshot beside its source without replacing any file.
    pub fn duplicate_note(
        &self,
        source: &Path,
        text: &str,
        reserved: &[PathBuf],
    ) -> Result<(PathBuf, SaveReceipt), VaultError> {
        self.path(source)?;
        let parent = source.parent().ok_or(VaultError::InvalidPath)?;
        let stem = source
            .file_stem()
            .ok_or(VaultError::InvalidPath)?
            .to_string_lossy();
        for serial in 1u64.. {
            let suffix = if serial == 1 {
                String::new()
            } else {
                format!(" {serial}")
            };
            let relative = parent.join(format!("{stem} 副本{suffix}.md"));
            if reserved.iter().any(|p| {
                p.to_string_lossy().replace('\\', "/").to_lowercase()
                    == relative.to_string_lossy().replace('\\', "/").to_lowercase()
            }) {
                continue;
            }
            match fs::symlink_metadata(self.path(&relative)?) {
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            match self.create(&relative, text) {
                Ok(receipt) => return Ok((relative, receipt)),
                Err(VaultError::Conflict { .. }) => continue,
                Err(VaultError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(VaultError::InvalidPath)
    }
    pub fn rename_folder(&self, old: &Path, new: &Path) -> Result<(), VaultError> {
        if new.starts_with(old) {
            return Err(VaultError::InvalidPath);
        }
        let source = self.folder_path(old)?;
        let dest = self.folder_path(new)?;
        fs::create_dir_all(dest.parent().ok_or(VaultError::InvalidPath)?)?;
        history::links::rename(self, old, new, &source, &dest)?;
        Ok(())
    }
    pub fn trash_folder(&self, relative: &Path) -> Result<PathBuf, VaultError> {
        let source = self.folder_path(relative)?;
        let trash = self.root.join(".inkstone-trash");
        if let Ok(meta) = fs::symlink_metadata(&trash)
            && is_reparse(&meta)
        {
            return Err(VaultError::InvalidPath);
        }
        fs::create_dir_all(&trash)?;
        let entry = trash.join(unique_id());
        fs::create_dir(&entry)?;
        write_new_synced(
            &entry.join("original-path.json"),
            &serde_json::to_vec(relative).map_err(io::Error::other)?,
        )?;
        let dest = entry.join(relative.file_name().ok_or(VaultError::InvalidPath)?);
        move_no_replace(&source, &dest)?;
        Ok(dest)
    }
    pub fn store_attachment_to(
        &self,
        folder: &Path,
        name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, VaultError> {
        self.write_attachment(folder, name, |file| file.write_all(bytes))
    }
    pub fn import_attachment_to(
        &self,
        folder: &Path,
        source: &Path,
    ) -> Result<PathBuf, VaultError> {
        let mut input = fs::File::open(source)?;
        if !input.metadata()?.is_file() {
            return Err(VaultError::InvalidPath);
        }
        let name = source
            .file_name()
            .ok_or(VaultError::InvalidPath)?
            .to_string_lossy();
        self.write_attachment(folder, &name, |file| io::copy(&mut input, file).map(|_| ()))
    }
    fn write_attachment(
        &self,
        folder: &Path,
        name: &str,
        write: impl FnOnce(&mut fs::File) -> io::Result<()>,
    ) -> Result<PathBuf, VaultError> {
        let name = Path::new(name);
        if name.components().count() != 1
            || !matches!(name.components().next(), Some(Component::Normal(_)))
            || name.to_string_lossy().contains(':')
        {
            return Err(VaultError::InvalidPath);
        }
        let dir = if folder.as_os_str().is_empty() {
            self.root.clone()
        } else {
            self.folder_path(folder)?
        };
        fs::create_dir_all(&dir)?;
        let stem = name.file_stem().unwrap_or_default().to_string_lossy();
        let extension = name
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        let mut serial = 0;
        let (path, mut file) = loop {
            let path = dir.join(if serial == 0 {
                name.to_path_buf()
            } else {
                PathBuf::from(format!("{stem} {serial}{extension}"))
            });
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => serial += 1,
                Err(e) => return Err(e.into()),
            }
        };
        if let Err(error) = write(&mut file).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        Ok(path
            .strip_prefix(&self.root)
            .map_err(|_| VaultError::InvalidPath)?
            .to_path_buf())
    }
    /// Validate every component, including existing junctions, before folder operations.
    pub fn folder_path(&self, relative: &Path) -> Result<PathBuf, VaultError> {
        if relative.as_os_str().is_empty()
            || relative.components().any(|c| {
                !matches!(c, Component::Normal(_))
                    || c.as_os_str().to_string_lossy().starts_with('.')
                    || c.as_os_str().to_string_lossy().contains(':')
            })
        {
            return Err(VaultError::InvalidPath);
        }
        let mut path = self.root.clone();
        for c in relative.components() {
            path.push(c);
            match fs::symlink_metadata(&path) {
                Ok(m) if is_reparse(&m) || !m.is_dir() => return Err(VaultError::InvalidPath),
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(path)
    }
    pub fn create_folder(&self, relative: &Path) -> Result<(), VaultError> {
        let path = self.folder_path(relative)?;
        fs::create_dir_all(path.parent().ok_or(VaultError::InvalidPath)?)?;
        fs::create_dir(path)?;
        Ok(())
    }
    pub fn folders(&self) -> Result<Vec<PathBuf>, VaultError> {
        Ok(self.scan_tree()?.folders)
    }
    pub fn trash_entries(&self) -> Result<Vec<TrashEntry>, VaultError> {
        let trash = self.root.join(".inkstone-trash");
        if !trash.exists() {
            return Ok(vec![]);
        }
        if is_reparse(&fs::symlink_metadata(&trash)?) {
            return Err(VaultError::InvalidPath);
        }
        let mut entries = vec![];
        for entry in fs::read_dir(trash)? {
            let dir = entry?.path();
            if is_reparse(&fs::symlink_metadata(&dir)?) || !dir.is_dir() {
                continue;
            }
            let Ok(bytes) = fs::read(dir.join("original-path.json")) else {
                continue;
            };
            let Ok(original) = serde_json::from_slice::<PathBuf>(&bytes) else {
                continue;
            };
            let stored =
                if fs::read(dir.join("payload-name.json")).is_ok_and(|b| b == b"\"content\"") {
                    dir.join("content")
                } else {
                    dir.join(original.file_name().ok_or(VaultError::InvalidPath)?)
                };
            if stored.exists() && !is_reparse(&fs::symlink_metadata(&stored)?) {
                let directory = stored.is_dir();
                if directory {
                    if original.as_os_str().is_empty()
                        || original.components().any(|c| {
                            !matches!(c, Component::Normal(_))
                                || c.as_os_str().to_string_lossy().contains(':')
                        })
                    {
                        return Err(VaultError::InvalidPath);
                    }
                } else {
                    self.regular_file_path(&original)?;
                }
                entries.push(TrashEntry {
                    stored,
                    original,
                    directory,
                });
            }
        }
        entries.sort_by(|a, b| b.stored.cmp(&a.stored));
        Ok(entries)
    }
    pub fn restore_trash(&self, entry: &TrashEntry) -> Result<(), VaultError> {
        // Re-enumerate so callers cannot supply a source outside the owned trash.
        if !self.trash_entries()?.iter().any(|e| {
            e.stored == entry.stored
                && e.original == entry.original
                && e.directory == entry.directory
        }) {
            return Err(VaultError::InvalidPath);
        }
        let dest = if entry.directory {
            self.folder_path(&entry.original)?
        } else {
            self.regular_file_path(&entry.original)?
        };
        fs::create_dir_all(dest.parent().ok_or(VaultError::InvalidPath)?)?;
        move_no_replace(&entry.stored, &dest)?;
        Ok(())
    }
    pub fn recoveries(&self) -> Result<Vec<RecoveryEntry>, VaultError> {
        let mut entries = vec![];
        for entry in fs::read_dir(&self.recovery_dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !is_reparse(&metadata) => metadata,
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                // A completed save can rename a journal after enumeration.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let record: Recovery = match serde_json::from_slice(&bytes) {
                Ok(record) => record,
                Err(_) => continue, // A partial crash journal remains on disk for manual inspection.
            };
            if record.root == self.root {
                entries.push(RecoveryEntry {
                    journal: path,
                    record,
                    modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                    bytes: bytes.len() as u64,
                });
            }
        }
        entries.sort_by_key(|e| e.modified);
        entries.reverse();
        Ok(entries)
    }

    /// Bound successful history only. Failed journals and conflict backups are never pruned.
    pub fn cleanup_history(&self) -> io::Result<()> {
        let policy = fs::read(self.root.join(".inkstone-workspace.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("history").cloned())
            .and_then(|value| serde_json::from_value::<history::Retention>(value).ok())
            .unwrap_or_default();
        self.cleanup_history_with_policy(
            if policy.days == 0 {
                std::time::Duration::MAX
            } else {
                std::time::Duration::from_secs(policy.days.saturating_mul(24 * 3600))
            },
            if policy.max_mib == 0 {
                u64::MAX
            } else {
                policy.max_mib.saturating_mul(1024 * 1024)
            },
        )
    }
    fn cleanup_history_with_policy(
        &self,
        max_age: std::time::Duration,
        max_bytes: u64,
    ) -> io::Result<()> {
        let mut saved = Vec::new();
        let mut protected = std::collections::BTreeSet::new();
        for entry in fs::read_dir(&self.recovery_dir)? {
            let path = entry?.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || is_reparse(&meta) {
                continue;
            }
            let completed = path.extension().is_some_and(|e| e == "saved");
            if !completed && path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<Recovery>(&bytes) else {
                continue;
            };
            if record.root != self.root {
                continue;
            }
            let Ok(note_path) = self.path(&record.relative) else {
                continue;
            };
            if !completed {
                protected.insert(record.relative);
                continue;
            }
            let backup = note_path.parent().unwrap().join(format!(
                ".inkstone-{}.saved-backup",
                path.file_stem().unwrap().to_string_lossy()
            ));
            let backup_meta = fs::symlink_metadata(&backup).ok();
            let regular_backup = backup_meta
                .as_ref()
                .is_some_and(|m| m.is_file() && !is_reparse(m));
            let backup_size = backup_meta
                .as_ref()
                .filter(|_| regular_backup)
                .map_or(0, |m| m.len());
            if backup_meta.is_some() && !regular_backup {
                protected.insert(record.relative.clone());
            }
            if regular_backup {
                // A Unix writer can keep an old inode open after the swap. Its
                // later changes are external data, even though our save succeeded.
                // Preserve this note's history if the displaced bytes changed.
                let unchanged = record.baseline.as_ref().is_some_and(|baseline| {
                    backup_size == baseline.len() as u64
                        && fs::read(&backup).is_ok_and(|bytes| bytes == baseline.as_bytes())
                });
                if !unchanged {
                    protected.insert(record.relative.clone());
                }
            }
            saved.push((
                path,
                record.relative,
                meta.modified()?,
                meta.len().saturating_add(backup_size),
                backup,
            ));
        }
        saved.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.0.cmp(&a.0)));
        let mut total = saved.iter().fold(0u64, |n, e| n.saturating_add(e.3));
        let mut latest = std::collections::BTreeSet::new();
        let mut candidates = Vec::new();
        for entry in saved {
            if !latest.insert(entry.1.clone()) && !protected.contains(&entry.1) {
                candidates.push(entry);
            }
        }
        // Remove oldest first, retaining at least the latest successful version of every note.
        for (journal, _, modified, size, backup) in candidates.into_iter().rev() {
            if total <= max_bytes && modified.elapsed().unwrap_or_default() <= max_age {
                continue;
            }
            match fs::symlink_metadata(&backup) {
                Ok(meta) if meta.is_file() && !is_reparse(&meta) => fs::remove_file(&backup)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
                // Never follow or remove unexpected entries.
                Ok(_) => continue,
            }
            match fs::remove_file(&journal) {
                Ok(()) => total = total.saturating_sub(size),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            history::records::forget(self, &journal);
        }
        history::records::prune_orphans(self);
        Ok(())
    }
    fn cleanup_history_if_due(&self) {
        static LAST_CLEANUP: std::sync::Mutex<Option<std::time::Instant>> =
            std::sync::Mutex::new(None);
        let Ok(mut last) = LAST_CLEANUP.lock() else {
            return;
        };
        if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(60)) {
            return;
        }
        *last = Some(std::time::Instant::now());
        drop(last);
        // Maintenance failure must never turn a successful note save into an error.
        let _ = self.cleanup_history();
    }

    pub fn rename_note(&self, old: &Path, new: &Path, baseline: &str) -> Result<(), VaultError> {
        let source = self.path(old)?;
        let dest = self.path(new)?;
        fs::create_dir_all(dest.parent().ok_or(VaultError::InvalidPath)?)?;
        let recovery = self.journal(old, Some(baseline), baseline)?;
        if read_optional(&source)?.as_deref() != Some(baseline) {
            return Err(VaultError::Conflict { recovery });
        }
        history::links::rename(self, old, new, &source, &dest)?;
        let _ = fs::rename(&recovery, recovery.with_extension("saved"));
        Ok(())
    }

    /// Deletion is a same-volume move into a visible-on-demand recoverable trash.
    pub fn trash_note(&self, relative: &Path, baseline: &str) -> Result<PathBuf, VaultError> {
        let source = self.path(relative)?;
        let recovery = self.journal(relative, Some(baseline), baseline)?;
        if read_optional(&source)?.as_deref() != Some(baseline) {
            return Err(VaultError::Conflict { recovery });
        }
        let trash = self.root.join(".inkstone-trash");
        if let Ok(meta) = fs::symlink_metadata(&trash)
            && is_reparse(&meta)
        {
            return Err(VaultError::InvalidPath);
        }
        fs::create_dir_all(&trash)?;
        let entry = trash.join(unique_id());
        fs::create_dir(&entry)?;
        write_new_synced(
            &entry.join("original-path.json"),
            &serde_json::to_vec(relative).map_err(io::Error::other)?,
        )?;
        let dest = entry.join(relative.file_name().ok_or(VaultError::InvalidPath)?);
        move_no_replace(&source, &dest)?;
        let _ = fs::rename(&recovery, recovery.with_extension("saved"));
        Ok(dest)
    }
    pub fn open(
        root: impl AsRef<Path>,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<Self, VaultError> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(VaultError::InvalidPath);
        }
        fs::create_dir_all(&recovery_dir)?;
        let recovery_dir = fs::canonicalize(recovery_dir)?;
        if recovery_dir.starts_with(&root) {
            return Err(VaultError::InvalidPath);
        }
        Ok(Self { root, recovery_dir })
    }
    fn validate_relative(relative: &Path) -> Result<(), VaultError> {
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            || relative
                .extension()
                .is_none_or(|x| !x.eq_ignore_ascii_case("md"))
        {
            return Err(VaultError::InvalidPath);
        }
        #[cfg(windows)]
        {
            if relative
                .components()
                .any(|c| c.as_os_str().to_string_lossy().contains(':'))
            {
                return Err(VaultError::InvalidPath);
            }
        }
        Ok(())
    }
    pub fn path(&self, relative: &Path) -> Result<PathBuf, VaultError> {
        Self::validate_relative(relative)?;
        self.regular_file_path(relative)
    }
    fn regular_file_path(&self, relative: &Path) -> Result<PathBuf, VaultError> {
        if relative.as_os_str().is_empty()
            || relative.components().any(|c| {
                !matches!(c, Component::Normal(_))
                    || (cfg!(windows) && c.as_os_str().to_string_lossy().contains(':'))
            })
        {
            return Err(VaultError::InvalidPath);
        }
        let mut path = self.root.clone();
        for component in relative.components() {
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(meta) if is_reparse(&meta) => return Err(VaultError::InvalidPath),
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(path)
    }
    pub fn read(&self, relative: &Path) -> Result<Option<String>, VaultError> {
        read_optional(&self.path(relative)?)
    }
    /// Read index content and timestamps from one validated, opened file.
    pub(crate) fn read_indexed(
        &self,
        relative: &Path,
    ) -> Result<Option<(String, crate::file_order::FileTimes)>, VaultError> {
        let mut file = match fs::File::open(self.path(relative)?) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let metadata = file.metadata()?;
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.read_to_end(&mut bytes)?;
        let text = String::from_utf8(bytes).map_err(|_| VaultError::InvalidUtf8)?;
        Ok(Some((text, (&metadata).into())))
    }
    pub fn scan(&self) -> Result<Vec<PathBuf>, VaultError> {
        Ok(self
            .scan_files()?
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .collect())
    }
    /// One directory walk for both indexed files and visible folders.
    pub fn scan_tree(&self) -> Result<VaultTree, VaultError> {
        fn walk(root: &Path, dir: &Path, tree: &mut VaultTree) -> io::Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let name = entry.file_name();
                // Workspace settings and recovery metadata are not vault assets.
                // Match the watcher's exclusion so saving settings cannot change
                // the indexed file list during a structural refresh.
                if name.to_string_lossy().starts_with(".inkstone-") {
                    continue;
                }
                let path = entry.path();
                let meta = fs::symlink_metadata(&path)?;
                if is_reparse(&meta) {
                    continue;
                }
                if meta.is_dir() {
                    if name.to_string_lossy().starts_with('.') {
                        continue;
                    }
                    tree.folders
                        .push(path.strip_prefix(root).unwrap().to_path_buf());
                    walk(root, &path, tree)?;
                } else if meta.is_file() {
                    tree.files
                        .push(path.strip_prefix(root).unwrap().to_path_buf());
                }
            }
            Ok(())
        }
        let mut tree = VaultTree::default();
        walk(&self.root, &self.root, &mut tree)?;
        tree.files.sort();
        tree.folders.sort();
        Ok(tree)
    }
    pub fn scan_files(&self) -> Result<Vec<PathBuf>, VaultError> {
        Ok(self.scan_tree()?.files)
    }
    pub fn journal(
        &self,
        relative: &Path,
        baseline: Option<&str>,
        draft: &str,
    ) -> Result<PathBuf, VaultError> {
        Self::validate_relative(relative)?;
        let path = self.recovery_dir.join(format!("{}.json", unique_id()));
        let record = Recovery {
            root: self.root.clone(),
            relative: relative.into(),
            baseline: baseline.map(str::to_owned),
            draft: draft.into(),
        };
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        write_new_synced(&path, &bytes)?;
        history::cache_journal(self, &path, &record);
        Ok(path)
    }
    pub fn save(
        &self,
        relative: &Path,
        baseline: Option<&str>,
        draft: &str,
    ) -> Result<SaveReceipt, VaultError> {
        self.save_with_hook(relative, baseline, draft, || {})
    }
    pub fn create(&self, relative: &Path, draft: &str) -> Result<SaveReceipt, VaultError> {
        let journal = self.journal(relative, None, draft)?;
        let path = self.path(relative)?;
        fs::create_dir_all(path.parent().ok_or(VaultError::InvalidPath)?)?;
        let receipt = self.save(relative, None, draft)?;
        let _ = fs::rename(&journal, journal.with_extension("saved"));
        Ok(receipt)
    }
    fn save_with_hook(
        &self,
        relative: &Path,
        baseline: Option<&str>,
        draft: &str,
        before_replace: impl FnOnce(),
    ) -> Result<SaveReceipt, VaultError> {
        let recovery = self.journal(relative, baseline, draft)?;
        let path = self.path(relative)?;
        if read_optional(&path)?.as_deref() != baseline {
            return Err(VaultError::Conflict { recovery });
        }
        #[cfg(any(windows, unix))]
        let guard = if baseline.is_some() {
            Some(SaveGuard::open(&path)?)
        } else {
            None
        };
        let parent = path.parent().ok_or(VaultError::InvalidPath)?;
        let temp = parent.join(format!(".inkstone-{}.tmp", unique_id()));
        write_new_synced(&temp, draft.as_bytes())?;
        let result = (|| {
            // Preserve Unix access permissions when replacing the inode.
            #[cfg(unix)]
            if let Some(guard) = &guard {
                fs::set_permissions(&temp, guard.0.metadata()?.permissions())?;
                fs::File::open(&temp)?.sync_all()?;
            }
            // Check again after writing the temporary file.
            if read_optional(&path)?.as_deref() != baseline {
                return Err(VaultError::Conflict {
                    recovery: recovery.clone(),
                });
            }
            before_replace();
            if baseline.is_none() {
                // Atomic no-clobber creation; an intervening creation is never overwritten.
                fs::hard_link(&temp, &path)?;
            } else {
                let backup = parent.join(format!(
                    ".inkstone-{}.external-backup",
                    recovery.file_stem().unwrap().to_string_lossy()
                ));
                replace_with_backup(&path, &temp, &backup)?;
                #[cfg(any(windows, unix))]
                if !guard
                    .as_ref()
                    .expect("existing target guard")
                    .matches(&backup)?
                {
                    return Err(VaultError::RaceConflict {
                        recovery: recovery.clone(),
                        external_backup: backup,
                    });
                }
                // Detect the compare/replace race and keep the captured external version.
                if read_optional(&backup)?.as_deref() != baseline {
                    return Err(VaultError::RaceConflict {
                        recovery: recovery.clone(),
                        external_backup: backup,
                    });
                }
                // The baseline and draft are already in the synced application journal.
                #[cfg(windows)]
                fs::remove_file(backup)?;
                #[cfg(unix)]
                move_no_replace(&backup, &backup.with_extension("saved-backup"))?;
            }
            #[cfg(not(windows))]
            {
                fs::File::open(parent)?.sync_all()?;
            }
            // Successful journals are retained separately, never offered as crash drafts.
            let committed = recovery.with_extension("saved");
            let recovery = if fs::rename(&recovery, &committed).is_ok() {
                committed
            } else {
                recovery.clone()
            };
            Ok(SaveReceipt {
                text: draft.into(),
                recovery,
            })
        })();
        // Best-effort removal only of our unique temporary file. Never touch a note on failure.
        let _ = fs::remove_file(&temp);
        if result.is_ok() {
            self.cleanup_history_if_due();
        }
        result
    }
}

#[cfg(any(windows, unix))]
struct SaveGuard(fs::File);
#[cfg(unix)]
impl SaveGuard {
    fn open(path: &Path) -> io::Result<Self> {
        fs::File::open(path).map(Self)
    }
    fn matches(&self, path: &Path) -> io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        let before = self.0.metadata()?;
        let displaced = fs::symlink_metadata(path)?;
        Ok(before.dev() == displaced.dev() && before.ino() == displaced.ino())
    }
}
#[cfg(windows)]
impl SaveGuard {
    fn open(path: &Path) -> io::Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(path)
            .map(Self)
    }
    fn identity(file: &fs::File) -> io::Result<(u32, u32, u32)> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    fn matches(&self, path: &Path) -> io::Result<bool> {
        Ok(Self::identity(&self.0)? == Self::identity(&fs::File::open(path)?)?)
    }
}

#[cfg(windows)]
fn move_no_replace(source: &Path, dest: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileW(source.as_ptr(), dest.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn move_no_replace(source: &Path, dest: &Path) -> io::Result<()> {
    rename_atomic(source, dest, false)
}

#[cfg(windows)]
fn replace_with_backup(path: &Path, temp: &Path, backup: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    let (path, temp, backup) = (wide(path), wide(temp), wide(backup));
    // Use the OS replacement operation with a backup. Documented partial failures
    // are recoverable from that backup plus the synced application journal.
    let ok = unsafe {
        ReplaceFileW(
            path.as_ptr(),
            temp.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace_with_backup(path: &Path, temp: &Path, backup: &Path) -> io::Result<()> {
    replace_with_backup_hook(path, temp, backup, || {})
}

#[cfg(not(windows))]
fn replace_with_backup_hook(
    path: &Path,
    temp: &Path,
    backup: &Path,
    before_exchange: impl FnOnce(),
) -> io::Result<()> {
    // Stage the draft at the backup name, then atomically exchange both inodes.
    // The backup captures exactly the file displaced by this operation, even if
    // another editor replaces the target after the last baseline check.
    move_no_replace(temp, backup)?;
    before_exchange();
    rename_atomic(backup, path, true)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_atomic(source: &Path, dest: &Path, exchange: bool) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let dest = CString::new(dest.as_os_str().as_bytes())?;
    // SAFETY: Both C strings are NUL-terminated and live through the call.
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            dest.as_ptr(),
            if exchange {
                libc::RENAME_SWAP
            } else {
                libc::RENAME_EXCL
            },
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            dest.as_ptr(),
            if exchange {
                libc::RENAME_EXCHANGE
            } else {
                libc::RENAME_NOREPLACE
            },
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn rename_atomic(_: &Path, _: &Path, _: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "此平台尚不支持安全文件替换",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_watcher_reports_external_note_changes() {
        use notify::Watcher;
        let s = Sandbox::new();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .unwrap();
        watcher
            .watch(&s.1.root, notify::RecursiveMode::Recursive)
            .unwrap();
        fs::write(s.1.root.join("external.md"), "外部新文档").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let event: notify::Event = receiver
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("external file event within 5 seconds")
                .unwrap();
            if event
                .paths
                .iter()
                .any(|path| path.strip_prefix(&s.1.root).ok() == Some(Path::new("external.md")))
            {
                break;
            }
        }
    }
    struct Sandbox(PathBuf, Vault);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("inkstone-test-{}", unique_id()));
            fs::create_dir_all(root.join("vault")).unwrap();
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            Self(root, vault)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn safe_save_reopen_preserves_exact_markdown() {
        let s = Sandbox::new();
        let note = Path::new("中文.md");
        let first = "# 标题\r\n\r\n**原始**  格式😀\r\n";
        s.1.save(note, None, first).unwrap();
        s.1.save(note, Some(first), "新内容\n").unwrap();
        assert_eq!(s.1.read(note).unwrap().as_deref(), Some("新内容\n"));
        assert_eq!(s.1.scan().unwrap(), vec![note.to_path_buf()]);
    }
    #[test]
    fn nested_note_creation_is_no_clobber() {
        let s = Sandbox::new();
        let note = Path::new("目录/子目录/笔记.md");
        s.1.create(note, "新内容").unwrap();
        assert_eq!(s.1.read(note).unwrap().unwrap(), "新内容");
        assert!(s.1.create(note, "不能覆盖").is_err());
        assert_eq!(s.1.read(note).unwrap().unwrap(), "新内容");
    }
    #[test]
    fn attachments_can_use_root_or_nested_folder_without_clobbering() {
        let s = Sandbox::new();
        assert_eq!(
            s.1.store_attachment_to(Path::new(""), "image.png", b"first")
                .unwrap(),
            Path::new("image.png")
        );
        assert_eq!(
            s.1.store_attachment_to(Path::new(""), "image.png", b"second")
                .unwrap(),
            Path::new("image 1.png")
        );
        let nested =
            s.1.store_attachment_to(Path::new("notes/assets"), "image.png", b"nested")
                .unwrap();
        assert_eq!(fs::read(s.1.root.join(nested)).unwrap(), b"nested");
        assert_eq!(fs::read(s.1.root.join("image.png")).unwrap(), b"first");
        assert!(
            s.1.store_attachment_to(Path::new("../outside"), "image.png", b"bad")
                .is_err()
        );
    }

    #[test]
    fn duplication_preserves_source_and_skips_disk_and_open_document_names() {
        let s = Sandbox::new();
        let source = Path::new("资料/笔记.2026.md");
        s.1.create(source, "磁盘原文").unwrap();
        s.1.create(Path::new("资料/笔记.2026 副本.md"), "不能覆盖")
            .unwrap();
        let reserved = vec![PathBuf::from("资料/笔记.2026 副本 2.md")];
        let draft = "# 新编辑😀\r\n[[目标]]";
        let (path, receipt) = s.1.duplicate_note(source, draft, &reserved).unwrap();
        assert_eq!(path, Path::new("资料/笔记.2026 副本 3.md"));
        assert_eq!(receipt.text, draft);
        assert_eq!(s.1.read(source).unwrap().as_deref(), Some("磁盘原文"));
        assert_eq!(
            s.1.read(Path::new("资料/笔记.2026 副本.md"))
                .unwrap()
                .as_deref(),
            Some("不能覆盖")
        );
        assert_eq!(s.1.read(&path).unwrap().as_deref(), Some(draft));
        assert!(
            s.1.duplicate_note(Path::new("../bad.md"), draft, &[])
                .is_err()
        );
    }
    #[test]
    fn folders_and_trash_restore_are_confined_and_no_clobber() {
        let s = Sandbox::new();
        s.1.create_folder(Path::new("空目录/子目录")).unwrap();
        assert!(
            s.1.folders()
                .unwrap()
                .contains(&PathBuf::from("空目录/子目录"))
        );
        assert!(s.1.create_folder(Path::new("../outside")).is_err());
        assert!(s.1.create_folder(Path::new(".inkstone-trash")).is_err());
        s.1.create(Path::new("空目录/a.md"), "原文😀").unwrap();
        s.1.trash_note(Path::new("空目录/a.md"), "原文😀").unwrap();
        let entries = s.1.trash_entries().unwrap();
        assert_eq!(entries.len(), 1);
        s.1.create(Path::new("空目录/a.md"), "新文件").unwrap();
        assert!(s.1.restore_trash(&entries[0]).is_err());
        assert_eq!(
            s.1.read(Path::new("空目录/a.md")).unwrap().unwrap(),
            "新文件"
        );
        s.1.rename_note(Path::new("空目录/a.md"), Path::new("新目录/b.md"), "新文件")
            .unwrap();
        s.1.restore_trash(&entries[0]).unwrap();
        assert_eq!(
            s.1.read(Path::new("空目录/a.md")).unwrap().unwrap(),
            "原文😀"
        );
        assert!(s.1.trash_entries().unwrap().is_empty());
        assert!(
            s.1.restore_trash(&TrashEntry {
                stored: s.0.join("anything.md"),
                directory: false,
                original: PathBuf::from("escape.md")
            })
            .is_err()
        );
    }
    #[test]
    fn attachment_collisions_preserve_existing_bytes_and_folder_trash_restores() {
        let s = Sandbox::new();
        let first =
            s.1.store_attachment_to(Path::new("附件"), "图片.png", b"first")
                .unwrap();
        let second =
            s.1.store_attachment_to(Path::new("附件"), "图片.png", b"second")
                .unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(s.1.root.join(&first)).unwrap(), b"first");
        assert!(
            s.1.store_attachment_to(Path::new("附件"), "../escape.png", b"x")
                .is_err()
        );
        s.1.rename_folder(Path::new("附件"), Path::new("资料/附件"))
            .unwrap();
        s.1.trash_folder(Path::new("资料")).unwrap();
        let entries = s.1.trash_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].directory);
        s.1.restore_trash(&entries[0]).unwrap();
        assert_eq!(
            fs::read(s.1.root.join("资料").join(first)).unwrap(),
            b"first"
        );
    }
    #[cfg(unix)]
    #[test]
    fn atomic_exchange_captures_a_replacement_after_draft_staging() {
        let s = Sandbox::new();
        let path = s.1.root.join("note.md");
        let temp = s.1.root.join("draft.tmp");
        let backup = s.1.root.join("backup");
        fs::write(&path, "old").unwrap();
        fs::write(&temp, "draft").unwrap();
        replace_with_backup_hook(&path, &temp, &backup, || {
            let external = s.1.root.join("external.tmp");
            fs::write(&external, "latest external").unwrap();
            fs::rename(external, &path).unwrap();
        })
        .unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "draft");
        assert_eq!(fs::read_to_string(backup).unwrap(), "latest external");
    }

    #[cfg(unix)]
    #[test]
    fn saving_preserves_private_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let s = Sandbox::new();
        let note = Path::new("private.md");
        s.1.create(note, "old").unwrap();
        fs::set_permissions(s.1.root.join(note), fs::Permissions::from_mode(0o600)).unwrap();
        s.1.save(note, Some("old"), "new").unwrap();
        assert_eq!(
            fs::metadata(s.1.root.join(note))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn folder_move_never_replaces_an_existing_empty_directory() {
        let s = Sandbox::new();
        s.1.create_folder(Path::new("source")).unwrap();
        s.1.create_folder(Path::new("destination")).unwrap();
        s.1.create(Path::new("source/note.md"), "keep").unwrap();
        assert!(
            s.1.rename_folder(Path::new("source"), Path::new("destination"))
                .is_err()
        );
        assert_eq!(
            s.1.read(Path::new("source/note.md")).unwrap().as_deref(),
            Some("keep")
        );
        assert!(s.1.root.join("destination").is_dir());
    }

    #[test]
    fn external_change_never_silently_overwrites() {
        let s = Sandbox::new();
        let note = Path::new("note.md");
        s.1.save(note, None, "old").unwrap();
        fs::write(s.1.path(note).unwrap(), "external").unwrap();
        let error = s.1.save(note, Some("old"), "draft").unwrap_err();
        let VaultError::Conflict { recovery } = error else {
            panic!("{error:?}")
        };
        let record: Recovery = serde_json::from_slice(&fs::read(recovery).unwrap()).unwrap();
        assert_eq!(record.draft, "draft");
        assert_eq!(s.1.read(note).unwrap().unwrap(), "external");
    }
    #[test]
    fn replacement_race_keeps_both_versions_and_reports_conflict() {
        let s = Sandbox::new();
        let note = Path::new("race.md");
        s.1.save(note, None, "old").unwrap();
        let error =
            s.1.save_with_hook(note, Some("old"), "draft", || {
                let replacement = s.1.root.join("racing.tmp");
                fs::write(&replacement, "racing edit").unwrap();
                fs::rename(replacement, s.1.path(note).unwrap()).unwrap();
            })
            .unwrap_err();
        let VaultError::RaceConflict {
            external_backup, ..
        } = error
        else {
            panic!("{error:?}")
        };
        assert_eq!(fs::read_to_string(external_backup).unwrap(), "racing edit");
        assert_eq!(s.1.read(note).unwrap().unwrap(), "draft");
    }
    #[test]
    fn create_race_does_not_clobber_another_file() {
        let s = Sandbox::new();
        let note = Path::new("new.md");
        assert!(
            s.1.save_with_hook(note, None, "draft", || fs::write(
                s.1.path(note).unwrap(),
                "other"
            )
            .unwrap())
                .is_err()
        );
        assert_eq!(s.1.read(note).unwrap().unwrap(), "other");
    }
    #[test]
    fn failed_save_retains_recovery_and_rejects_traversal() {
        let s = Sandbox::new();
        assert!(
            s.1.save(Path::new("absent/note.md"), None, "recover me")
                .is_err()
        );
        assert_eq!(s.1.recoveries().unwrap().len(), 1);
        assert!(s.1.path(Path::new("../escape.md")).is_err());
        assert!(s.1.path(Path::new("x.txt")).is_err());
    }
    #[test]
    fn rename_collision_and_recoverable_delete() {
        let s = Sandbox::new();
        s.1.save(Path::new("a.md"), None, "A").unwrap();
        s.1.save(Path::new("b.md"), None, "B").unwrap();
        assert!(
            s.1.rename_note(Path::new("a.md"), Path::new("b.md"), "A")
                .is_err()
        );
        assert_eq!(s.1.read(Path::new("b.md")).unwrap().unwrap(), "B");
        s.1.rename_note(Path::new("a.md"), Path::new("c.md"), "A")
            .unwrap();
        assert_eq!(s.1.read(Path::new("a.md")).unwrap(), None);
        let trash = s.1.trash_note(Path::new("c.md"), "A").unwrap();
        assert_eq!(fs::read_to_string(trash).unwrap(), "A");
        assert_eq!(s.1.scan().unwrap(), vec![PathBuf::from("b.md")]);
    }
    #[cfg(unix)]
    #[test]
    fn history_cleanup_keeps_late_external_writes_to_successful_backups() {
        let s = Sandbox::new();
        let note = Path::new("late.md");
        s.1.save(note, None, "one").unwrap();
        let writer_path = s.1.root.join(note);
        let mut writer = OpenOptions::new().write(true).open(writer_path).unwrap();
        let old = s.1.save(note, Some("one"), "two").unwrap().recovery;
        s.1.save(note, Some("two"), "three").unwrap();
        writer.write_all(b"external late edit").unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        let backup = s.1.root.join(format!(
            ".inkstone-{}.saved-backup",
            old.file_stem().unwrap().to_string_lossy()
        ));
        s.1.cleanup_history_with_policy(std::time::Duration::ZERO, 0)
            .unwrap();
        assert!(old.exists());
        assert_eq!(fs::read_to_string(backup).unwrap(), "external late edit");
        assert_eq!(s.1.read(note).unwrap().as_deref(), Some("three"));
    }

    #[test]
    fn history_cleanup_preserves_latest_failed_and_other_vault_records() {
        let s = Sandbox::new();
        let note = Path::new("note.md");
        s.1.save(note, None, "one").unwrap();
        let old = s.1.save(note, Some("one"), "two").unwrap().recovery;
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let latest = s.1.save(note, Some("two"), "three").unwrap().recovery;
        // Same recovery directory may contain another vault's records.
        fs::create_dir(s.0.join("other")).unwrap();
        let other = Vault::open(s.0.join("other"), &s.1.recovery_dir).unwrap();
        let other_record = other.save(note, None, "other").unwrap().recovery;
        let failure = s.1.journal(note, Some("three"), "unsaved").unwrap();
        let conflict_backup = s.1.root.join(".inkstone-unresolved.external-backup");
        fs::write(&conflict_backup, "external").unwrap();
        let corrupt = s.1.recovery_dir.join("partial.json");
        fs::write(&corrupt, "{").unwrap();
        s.1.cleanup_history_with_policy(std::time::Duration::ZERO, 0)
            .unwrap();
        assert!(
            old.exists(),
            "unresolved drafts protect this note's successful history"
        );
        fs::remove_file(failure).unwrap();
        s.1.cleanup_history_with_policy(std::time::Duration::ZERO, 0)
            .unwrap();
        assert!(!old.exists());
        assert!(latest.exists());
        assert!(other_record.exists());
        assert!(conflict_backup.exists());
        assert!(corrupt.exists());
        #[cfg(unix)]
        assert!(
            !s.1.root
                .join(format!(
                    ".inkstone-{}.saved-backup",
                    old.file_stem().unwrap().to_string_lossy()
                ))
                .exists()
        );
        assert_eq!(s.1.read(note).unwrap().as_deref(), Some("three"));
    }

    #[test]
    fn recovery_list_excludes_success_and_keeps_failed_draft() {
        let s = Sandbox::new();
        let note = Path::new("note.md");
        s.1.save(note, None, "first").unwrap();
        assert!(s.1.recoveries().unwrap().is_empty());
        assert!(s.1.save(note, Some("wrong"), "recover draft").is_err());
        let entries = s.1.recoveries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].record.draft, "recover draft");
    }
    #[cfg(any(windows, unix))]
    #[test]
    fn same_content_inode_swap_preserves_late_external_write() {
        #[cfg(windows)]
        use std::os::windows::fs::OpenOptionsExt;
        let s = Sandbox::new();
        let note = Path::new("swapped.md");
        s.1.save(note, None, "old").unwrap();
        let mut writer = None;
        let error =
            s.1.save_with_hook(note, Some("old"), "draft", || {
                let path = s.1.root.join("new-inode.tmp");
                fs::write(&path, "old").unwrap();
                let mut options = OpenOptions::new();
                options.write(true);
                #[cfg(windows)]
                options.share_mode(7);
                writer = Some(options.open(&path).unwrap());
                fs::rename(path, s.1.path(note).unwrap()).unwrap();
            })
            .unwrap_err();
        let VaultError::RaceConflict {
            external_backup, ..
        } = error
        else {
            panic!("{error:?}")
        };
        let mut writer = writer.unwrap();
        writer.write_all(b"ext").unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        assert_eq!(fs::read_to_string(external_backup).unwrap(), "ext");
        assert_eq!(s.1.read(note).unwrap().unwrap(), "draft");
    }
    #[cfg(windows)]
    #[test]
    fn existing_writer_handle_prevents_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let s = Sandbox::new();
        let note = Path::new("writer.md");
        s.1.save(note, None, "old").unwrap();
        let mut writer = OpenOptions::new()
            .write(true)
            .share_mode(7)
            .open(s.1.path(note).unwrap())
            .unwrap();
        assert!(s.1.save(note, Some("old"), "draft").is_err());
        writer.write_all(b"new").unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        assert_eq!(s.1.read(note).unwrap().unwrap(), "new");
        assert_eq!(s.1.recoveries().unwrap()[0].record.draft, "draft");
    }
    #[cfg(windows)]
    #[test]
    fn sharing_violation_preserves_original_and_draft() {
        use std::os::windows::fs::OpenOptionsExt;
        let s = Sandbox::new();
        let note = Path::new("locked.md");
        s.1.save(note, None, "old").unwrap();
        let _locked = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(s.1.path(note).unwrap())
            .unwrap();
        assert!(s.1.save(note, Some("old"), "draft").is_err());
        assert_eq!(s.1.read(note).unwrap().unwrap(), "old");
        assert_eq!(s.1.history(note).unwrap().len(), 2);
    }
}
