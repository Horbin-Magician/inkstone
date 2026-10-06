//! Versioned, checksummed directory backups. Publication/restoration never overwrites.
pub mod capacity;
pub mod cleanup;
mod locking;
mod origin;
pub mod retention;
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub created: u64,
    pub source_name: String,
    /// Filesystem source identity for conservative retention grouping. Old
    /// backups and sources without reliable identity remain unclassified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub directories: Vec<PathBuf>,
    pub files: Vec<Entry>,
}
impl Manifest {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|e| e.bytes).sum()
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn relative(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if path.to_string_lossy().contains(':') {
        return Err(invalid("备份中包含无效路径"));
    }
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid("备份中包含无效路径"));
    }
    Ok(())
}
pub(super) fn safe_path(root: &Path, path: &Path) -> io::Result<PathBuf> {
    relative(path)?;
    let mut full = root.to_owned();
    for component in path.components() {
        full.push(component);
        if is_reparse(&fs::symlink_metadata(&full)?) {
            return Err(invalid(format!(
                "不支持备份或恢复符号链接：{}",
                path.display()
            )));
        }
    }
    Ok(full)
}
fn inventory(root: &Path) -> io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    fn walk(
        root: &Path,
        directory: &Path,
        dirs: &mut Vec<PathBuf>,
        files: &mut Vec<PathBuf>,
    ) -> io::Result<()> {
        for item in fs::read_dir(directory)? {
            let path = item?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if is_reparse(&metadata) {
                return Err(invalid(format!("不支持符号链接：{}", path.display())));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("路径越出备份范围"))?
                .to_owned();
            if metadata.is_dir() {
                dirs.push(relative);
                walk(root, &path, dirs, files)?;
            } else if metadata.is_file() {
                files.push(relative);
            } else {
                return Err(invalid(format!("不支持特殊文件：{}", path.display())));
            }
        }
        Ok(())
    }
    let (mut dirs, mut files) = (vec![], vec![]);
    walk(root, root, &mut dirs, &mut files)?;
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}
pub(super) fn transfer(source: &Path, destination: Option<&Path>) -> io::Result<(u64, String)> {
    let before = fs::symlink_metadata(source)?;
    if !before.is_file() || is_reparse(&before) {
        return Err(invalid("备份源不是普通文件"));
    }
    let mut input = fs::File::open(source)?;
    let mut output = destination
        .map(|p| OpenOptions::new().write(true).create_new(true).open(p))
        .transpose()?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        if let Some(output) = &mut output {
            output.write_all(&buffer[..n])?;
        }
        bytes = bytes
            .checked_add(n as u64)
            .ok_or_else(|| invalid("文件过大"))?;
    }
    let after = fs::symlink_metadata(source)?;
    if is_reparse(&after)
        || bytes != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid(format!(
            "复制期间文件发生变化：{}，请重试",
            source.display()
        )));
    }
    if let Some(output) = output {
        output.sync_all()?;
        output.set_permissions(before.permissions())?;
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}
pub(super) struct Staging(pub(super) PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(super) fn staging(destination: &Path, excluded: &Path) -> io::Result<(Staging, PathBuf)> {
    let parent = fs::canonicalize(
        destination
            .parent()
            .ok_or_else(|| invalid("请选择目标文件夹"))?,
    )?;
    let excluded = fs::canonicalize(excluded)?;
    if parent.starts_with(&excluded) {
        return Err(invalid("目标必须位于源文件夹之外"));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| invalid("无效目标名称"))?;
    let target = parent.join(name);
    if target.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "目标已存在，不会覆盖",
        ));
    }
    let temp = parent.join(format!(".inkstone-backup-staging-{}", unique_id()));
    fs::create_dir(&temp)?;
    Ok((Staging(temp), target))
}

pub fn create(vault: &Vault, destination: &Path) -> io::Result<Manifest> {
    let _lock = locking::acquire(
        destination
            .parent()
            .ok_or_else(|| invalid("无效备份位置"))?,
        true,
    )?;
    let (stage, destination) = staging(destination, &vault.root)?;
    let payload = stage.0.join("files");
    fs::create_dir(&payload)?;
    let (directories, files) = inventory(&vault.root)?;
    for dir in &directories {
        fs::create_dir_all(payload.join(dir))?;
    }
    let mut manifest = Manifest {
        version: 1,
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        source_name: vault
            .root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        source_id: origin::id(&vault.root)?,
        directories,
        files: vec![],
    };
    for path in &files {
        let (bytes, sha256) = transfer(&safe_path(&vault.root, path)?, Some(&payload.join(path)))?;
        manifest.files.push(Entry {
            path: path.clone(),
            bytes,
            sha256,
        });
    }
    if inventory(&vault.root)? != (manifest.directories.clone(), files) {
        return Err(invalid("备份期间笔记库目录发生变化，请重试"));
    }
    for entry in &manifest.files {
        if transfer(&safe_path(&vault.root, &entry.path)?, None)?
            != (entry.bytes, entry.sha256.clone())
        {
            return Err(invalid(format!(
                "备份期间内容发生变化：{}，请重试",
                entry.path.display()
            )));
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.0.join("manifest.json"))?;
    serde_json::to_writer_pretty(&mut file, &manifest)?;
    file.sync_all()?;
    drop(file);
    inspect_unlocked(&stage.0)?;
    move_no_replace(&stage.0, &destination)?;
    Ok(manifest)
}

fn inspect_metadata(source: &Path) -> io::Result<Manifest> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(invalid("备份位置必须是普通目录"));
    }
    let path = safe_path(source, Path::new("manifest.json"))?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(invalid("备份清单必须是普通文件"));
    }
    if metadata.len() > 16 * 1024 * 1024 {
        return Err(invalid("备份清单过大"));
    }
    let manifest: Manifest = serde_json::from_reader(fs::File::open(path)?)?;
    if manifest.version != 1 {
        return Err(invalid("不支持此备份格式版本"));
    }
    if manifest
        .source_id
        .as_ref()
        .is_some_and(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid("无效的备份来源标识"));
    }
    let mut paths = BTreeSet::new();
    for path in manifest
        .directories
        .iter()
        .chain(manifest.files.iter().map(|e| &e.path))
    {
        relative(path)?;
        if !paths.insert(path) {
            return Err(invalid("备份清单路径重复"));
        }
    }
    let mut total = 0u64;
    for entry in &manifest.files {
        if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("无效的内容校验值"));
        }
        total = total
            .checked_add(entry.bytes)
            .ok_or_else(|| invalid("备份大小溢出"))?;
    }
    let payload = safe_path(source, Path::new("files"))?;
    let (dirs, files) = inventory(&payload)?;
    let mut expected_dirs = manifest.directories.clone();
    expected_dirs.sort();
    let mut expected_files: Vec<_> = manifest.files.iter().map(|e| e.path.clone()).collect();
    expected_files.sort();
    if (dirs, files) != (expected_dirs, expected_files) {
        return Err(invalid("备份文件清单不完整或存在额外文件"));
    }
    for entry in &manifest.files {
        if fs::metadata(safe_path(&payload, &entry.path)?)?.len() != entry.bytes {
            return Err(invalid(format!(
                "备份文件大小不符：{}",
                entry.path.display()
            )));
        }
    }
    Ok(manifest)
}

pub fn inspect(source: &Path) -> io::Result<Manifest> {
    if is_reparse(&fs::symlink_metadata(source)?) {
        return Err(invalid("备份目录不能是符号链接"));
    }
    let source = fs::canonicalize(source)?;
    let _lock = locking::acquire(
        source.parent().ok_or_else(|| invalid("无效备份位置"))?,
        false,
    )?;
    inspect_unlocked(&source)
}

fn inspect_unlocked(source: &Path) -> io::Result<Manifest> {
    let manifest = inspect_metadata(source)?;
    let payload = safe_path(source, Path::new("files"))?;
    for entry in &manifest.files {
        if transfer(&safe_path(&payload, &entry.path)?, None)?
            != (entry.bytes, entry.sha256.clone())
        {
            return Err(invalid(format!("内容校验失败：{}", entry.path.display())));
        }
    }
    Ok(manifest)
}

pub fn restore(source: &Path, destination: &Path, expected: &Manifest) -> io::Result<Manifest> {
    let canonical = fs::canonicalize(source)?;
    let _lock = locking::acquire(
        canonical.parent().ok_or_else(|| invalid("无效备份位置"))?,
        false,
    )?;
    let manifest = inspect_unlocked(source)?;
    if &manifest != expected {
        return Err(invalid("备份在预览后发生变化，请重新选择并校验"));
    }
    let (stage, destination) = staging(destination, source)?;
    for dir in &manifest.directories {
        fs::create_dir_all(stage.0.join(dir))?;
    }
    for entry in &manifest.files {
        let path = safe_path(&source.join("files"), &entry.path)?;
        if transfer(&path, Some(&stage.0.join(&entry.path)))? != (entry.bytes, entry.sha256.clone())
        {
            return Err(invalid(format!(
                "恢复时内容校验失败：{}",
                entry.path.display()
            )));
        }
    }
    move_no_replace(&stage.0, &destination)?;
    Ok(manifest)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub directory: Option<PathBuf>,
    pub interval_hours: u32,
    pub last_success: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf, Vault);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("inkstone-backup-test-{}", unique_id()));
            fs::create_dir_all(root.join("vault/空目录")).unwrap();
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            fs::write(
                vault.root.join("笔记.md"),
                "# 中文\r\n😀 ![](image.bin)\r\n",
            )
            .unwrap();
            fs::write(vault.root.join("image.bin"), [0, 255, 128, 3]).unwrap();
            fs::write(vault.root.join(".inkstone-workspace.json"), "{}").unwrap();
            Self(root, vault)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn capacity_lists_metadata_without_treating_it_as_verified_content() {
        let f = Fixture::new();
        let destination = f.0.join("backups");
        fs::create_dir(&destination).unwrap();
        let first = destination.join("first");
        let second = destination.join("second");
        let manifest = create(&f.1, &first).unwrap();
        create(&f.1, &second).unwrap();
        let summary = capacity::summarize(&first).unwrap();
        assert_eq!(summary.payload_bytes, manifest.bytes());
        assert_eq!(summary.files, manifest.files.len());
        assert_eq!(summary.created, manifest.created);
        assert_eq!(summary.source_name, manifest.source_name);
        assert_eq!(
            summary.manifest_bytes,
            fs::metadata(first.join("manifest.json")).unwrap().len()
        );
        fs::write(first.join("files/image.bin"), [9, 8, 7, 6]).unwrap();
        assert_eq!(capacity::summarize(&first).unwrap(), summary);
        assert!(inspect(&first).is_err());
        let inventory = capacity::list(&destination).unwrap();
        assert_eq!(inventory.entries.len(), 2);
        assert_eq!(inventory.payload_bytes, manifest.bytes() * 2);
        assert_eq!(inventory.manifest_bytes, summary.manifest_bytes * 2);
        assert_eq!(inventory.unreadable, 0);
        fs::write(second.join("files/image.bin"), b"short").unwrap();
        fs::create_dir(destination.join("unrelated")).unwrap();
        fs::create_dir(destination.join(".inkstone-backup-staging-test")).unwrap();
        fs::create_dir(destination.join("inkstone-backup-broken")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&first, destination.join("linked")).unwrap();
        let inventory = capacity::list(&destination).unwrap();
        assert_eq!(inventory.entries, vec![summary]);
        assert_eq!(inventory.payload_bytes, manifest.bytes());
        assert_eq!(inventory.unreadable, 2 + usize::from(cfg!(unix)));
        assert_eq!(
            fs::read(first.join("files/image.bin")).unwrap(),
            [9, 8, 7, 6]
        );
    }

    #[test]
    fn backup_origin_distinguishes_same_names_and_preserves_legacy_restore() {
        let f = Fixture::new();
        let first = create(&f.1, &f.0.join("first")).unwrap();
        let repeated = create(&f.1, &f.0.join("repeat")).unwrap();
        assert_eq!(first.source_id, repeated.source_id);
        let other_root = f.0.join("other/vault");
        fs::create_dir_all(&other_root).unwrap();
        let other = Vault::open(&other_root, f.0.join("other-recovery")).unwrap();
        let second = create(&other, &f.0.join("second")).unwrap();
        assert_eq!(first.source_name, second.source_name);
        if fs::metadata(&f.1.root).unwrap().created().is_ok()
            && fs::metadata(&other.root).unwrap().created().is_ok()
        {
            assert!(first.source_id.is_some());
            assert_ne!(first.source_id, second.source_id);
        }
        fs::rename(&f.1.root, f.0.join("old-vault")).unwrap();
        fs::create_dir(&f.1.root).unwrap();
        let replacement = create(&f.1, &f.0.join("replacement")).unwrap();
        if first.source_id.is_some() && replacement.source_id.is_some() {
            assert_ne!(first.source_id, replacement.source_id);
        }
        let path = f.0.join("first/manifest.json");
        let mut legacy = serde_json::to_value(&first).unwrap();
        legacy.as_object_mut().unwrap().remove("source_id");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let legacy_manifest = inspect(&f.0.join("first")).unwrap();
        assert!(legacy_manifest.source_id.is_none());
        assert!(
            capacity::summarize(&f.0.join("first"))
                .unwrap()
                .source_id
                .is_none()
        );
        restore(&f.0.join("first"), &f.0.join("restored"), &legacy_manifest).unwrap();
        assert_eq!(
            fs::read(f.0.join("restored/image.bin")).unwrap(),
            [0, 255, 128, 3]
        );
        legacy["source_id"] = serde_json::Value::String("invalid".into());
        fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(inspect(&f.0.join("first")).is_err());
        assert!(capacity::summarize(&f.0.join("first")).is_err());
    }

    #[test]
    fn backup_operation_locks_protect_readers_and_coordinate_processes() {
        const CHILD: &str = "INKSTONE_BACKUP_LOCK_CHILD";
        if let Some(directory) = std::env::var_os(CHILD) {
            let error = locking::acquire(Path::new(&directory), true).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            return;
        }
        let f = Fixture::new();
        let path = f.0.join("backup");
        let manifest = create(&f.1, &path).unwrap();
        let reader = locking::acquire(&f.0, false).unwrap();
        let second_reader = locking::acquire(&f.0, false).unwrap();
        assert_eq!(inspect(&path).unwrap(), manifest);
        assert!(create(&f.1, &f.0.join("blocked")).is_err());
        assert!(!f.0.join("blocked").exists());
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "vault::backup::tests::backup_operation_locks_protect_readers_and_coordinate_processes"])
            .env(CHILD, &f.0).status().unwrap();
        assert!(status.success());
        drop(second_reader);
        drop(reader);
        let exclusive = locking::acquire(&f.0, true).unwrap();
        assert!(inspect(&path).is_err());
        assert!(restore(&path, &f.0.join("blocked-restore"), &manifest).is_err());
        assert!(!f.0.join("blocked-restore").exists());
        drop(exclusive);
        restore(&path, &f.0.join("restored"), &manifest).unwrap();
        create(&f.1, &f.0.join("unblocked")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_locks_do_not_require_writing_read_only_backup_media() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let media = f.0.join("media");
        fs::create_dir(&media).unwrap();
        let path = media.join("backup");
        let manifest = create(&f.1, &path).unwrap();
        fs::set_permissions(&media, fs::Permissions::from_mode(0o555)).unwrap();
        let result = restore(&path, &f.0.join("restored"), &manifest);
        fs::set_permissions(&media, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap(), manifest);
        assert_eq!(fs::read_dir(&media).unwrap().count(), 1);
    }

    #[test]
    fn cleanup_preflight_protects_retained_content_stale_plans_and_readers() {
        let f = Fixture::new();
        let parent = f.0.join("backups");
        fs::create_dir(&parent).unwrap();
        for created in 1..=2 {
            let path = parent.join(created.to_string());
            let mut manifest = create(&f.1, &path).unwrap();
            manifest.created = created;
            manifest.source_id = Some("a".repeat(64));
            fs::write(
                path.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let plan =
            retention::preview(&capacity::list(&parent).unwrap(), 1, &BTreeSet::new()).unwrap();
        assert_eq!(plan.candidates, 1);
        let reader = locking::acquire(&parent, false).unwrap();
        assert!(cleanup::prepare(&parent, &plan, &BTreeSet::new()).is_err());
        drop(reader);
        let checked = cleanup::prepare(&parent, &plan, &BTreeSet::new()).unwrap();
        assert_eq!(checked.preview(), &plan);
        assert!(inspect(&parent.join("1")).is_err());
        assert!(create(&f.1, &parent.join("blocked")).is_err());
        drop(checked);
        assert!(cleanup::prepare(&parent, &plan, &[parent.join("1")].into()).is_err());
        let retained = parent.join("2/files/image.bin");
        fs::write(&retained, [9, 8, 7, 6]).unwrap();
        assert_eq!(
            retention::preview(&capacity::list(&parent).unwrap(), 1, &BTreeSet::new()).unwrap(),
            plan
        );
        assert!(cleanup::prepare(&parent, &plan, &BTreeSet::new()).is_err());
        fs::write(&retained, [0, 255, 128, 3]).unwrap();
        fs::write(parent.join("1/extra.txt"), "must preserve").unwrap();
        assert!(cleanup::prepare(&parent, &plan, &BTreeSet::new()).is_err());
        fs::remove_file(parent.join("1/extra.txt")).unwrap();
        create(&f.1, &parent.join("new")).unwrap();
        assert!(cleanup::prepare(&parent, &plan, &BTreeSet::new()).is_err());
        for name in ["1", "2", "new"] {
            assert!(inspect(&parent.join(name)).is_ok());
        }
    }

    #[test]
    fn cleanup_execution_rejects_replaced_directories_and_keeps_protected_backups() {
        let f = Fixture::new();
        let parent = f.0.join("backups");
        fs::create_dir(&parent).unwrap();
        let mut old_manifest = None;
        for created in 0..=2 {
            let path = parent.join(created.to_string());
            let mut manifest = create(&f.1, &path).unwrap();
            manifest.created = created;
            manifest.source_id = (created > 0).then(|| "a".repeat(64));
            fs::write(
                path.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            if created == 1 {
                old_manifest = Some(manifest);
            }
        }
        let plan =
            retention::preview(&capacity::list(&parent).unwrap(), 1, &BTreeSet::new()).unwrap();
        let replacement = f.0.join("replacement");
        create(&f.1, &replacement).unwrap();
        fs::write(
            replacement.join("manifest.json"),
            serde_json::to_vec(&old_manifest.unwrap()).unwrap(),
        )
        .unwrap();
        let checked = cleanup::prepare(&parent, &plan, &BTreeSet::new()).unwrap();
        fs::rename(parent.join("1"), f.0.join("displaced")).unwrap();
        fs::rename(replacement, parent.join("1")).unwrap();
        assert!(checked.execute().is_err());
        assert!(inspect(&parent.join("1")).is_ok());
        let checked = cleanup::prepare(&parent, &plan, &BTreeSet::new()).unwrap();
        fs::write(parent.join("2/files/image.bin"), [9, 8, 7, 6]).unwrap();
        assert!(checked.execute().is_err());
        assert!(parent.join("1").exists());
        fs::write(parent.join("2/files/image.bin"), [0, 255, 128, 3]).unwrap();
        let report = cleanup::prepare(&parent, &plan, &BTreeSet::new())
            .unwrap()
            .execute()
            .unwrap();
        assert_eq!(
            (report.removed, report.logical_bytes),
            (1, plan.candidate_bytes)
        );
        assert!(!parent.join("1").exists());
        assert!(inspect(&parent.join("0")).is_ok());
        assert!(inspect(&parent.join("2")).is_ok());
        assert!(inspect(&f.0.join("displaced")).is_ok());
        assert_eq!(fs::read_dir(parent).unwrap().count(), 2);
        assert_eq!(
            fs::read(f.1.root.join("image.bin")).unwrap(),
            [0, 255, 128, 3]
        );
    }

    #[test]
    fn cleanup_quarantine_preserves_late_writes_and_never_overwrites_on_rollback() {
        for collision in [false, true] {
            let f = Fixture::new();
            let parent = f.0.join("backups");
            fs::create_dir(&parent).unwrap();
            for created in 1..=2 {
                let path = parent.join(created.to_string());
                let mut manifest = create(&f.1, &path).unwrap();
                manifest.created = created;
                manifest.source_id = Some("a".repeat(64));
                fs::write(
                    path.join("manifest.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
            }
            let plan =
                retention::preview(&capacity::list(&parent).unwrap(), 1, &BTreeSet::new()).unwrap();
            let mut isolated = PathBuf::new();
            let result = cleanup::prepare(&parent, &plan, &BTreeSet::new())
                .unwrap()
                .execute_with(|source, quarantine| {
                    fs::write(quarantine.join("files/image.bin"), b"late external write").unwrap();
                    isolated = quarantine.to_owned();
                    if collision {
                        fs::create_dir(source).unwrap();
                        fs::write(source.join("user.txt"), b"new occupant").unwrap();
                    }
                });
            assert!(result.is_err());
            if collision {
                assert_eq!(
                    fs::read(parent.join("1/user.txt")).unwrap(),
                    b"new occupant"
                );
                assert_eq!(
                    fs::read(isolated.join("files/image.bin")).unwrap(),
                    b"late external write"
                );
            } else {
                assert!(!isolated.exists());
                assert_eq!(
                    fs::read(parent.join("1/files/image.bin")).unwrap(),
                    b"late external write"
                );
            }
            assert!(inspect(&parent.join("2")).is_ok());
        }
    }

    #[test]
    fn interrupted_cleanup_is_visible_protected_and_restorable_after_process_exit() {
        const CHILD: &str = "INKSTONE_BACKUP_CLEANUP_EXIT";
        if let Some(root) = std::env::var_os(CHILD) {
            let parent = PathBuf::from(root).join("backups");
            let plan =
                retention::preview(&capacity::list(&parent).unwrap(), 1, &BTreeSet::new()).unwrap();
            cleanup::prepare(&parent, &plan, &BTreeSet::new())
                .unwrap()
                .execute_with(|_, _| std::process::exit(74))
                .unwrap();
            unreachable!();
        }
        let f = Fixture::new();
        let parent = f.0.join("backups");
        fs::create_dir(&parent).unwrap();
        for created in 1..=3 {
            let path = parent.join(created.to_string());
            let mut manifest = create(&f.1, &path).unwrap();
            manifest.created = created;
            manifest.source_id = Some("a".repeat(64));
            fs::write(
                path.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "vault::backup::tests::interrupted_cleanup_is_visible_protected_and_restorable_after_process_exit"])
            .env(CHILD, &f.0).stdout(std::process::Stdio::null()).status().unwrap();
        assert_eq!(status.code(), Some(74));
        let inventory = capacity::list(&parent).unwrap();
        assert_eq!(inventory.interrupted.len(), 1);
        assert_eq!(inventory.entries.len(), 2);
        let plan = retention::preview(&inventory, 1, &BTreeSet::new()).unwrap();
        assert_eq!(plan.candidates, 0);
        assert!(
            plan.records
                .iter()
                .all(|r| r.decision == retention::Decision::IncompleteInventory)
        );
        assert!(cleanup::prepare(&parent, &plan, &BTreeSet::new()).is_err());
        let isolated = &inventory.interrupted[0];
        let manifest = inspect(isolated).unwrap();
        restore(isolated, &f.0.join("restored-interrupted"), &manifest).unwrap();
        assert_eq!(
            fs::read(f.0.join("restored-interrupted/image.bin")).unwrap(),
            [0, 255, 128, 3]
        );
        fs::remove_file(isolated.join("manifest.json")).unwrap();
        assert!(inspect(isolated).is_err());
        assert_eq!(
            capacity::list(&parent).unwrap().interrupted,
            inventory.interrupted
        );
        assert!(isolated.join("files/image.bin").exists());
        assert!(inspect(&parent.join("3")).is_ok());
    }

    #[test]
    fn interrupted_backup_retention_is_no_clobber_and_protected_from_cleanup() {
        let f = Fixture::new();
        let parent = f.0.join("backups");
        fs::create_dir(&parent).unwrap();
        let isolated = parent.join(".inkstone-backup-cleanup-old");
        let newest = parent.join("newest");
        for (path, created) in [(&isolated, 1), (&newest, 2)] {
            let mut manifest = create(&f.1, path).unwrap();
            manifest.created = created;
            manifest.source_id = Some("a".repeat(64));
            fs::write(
                path.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let reader = locking::acquire(&parent, false).unwrap();
        assert!(cleanup::retain_interrupted(&parent, &isolated).is_err());
        drop(reader);
        let retained = cleanup::retain_interrupted(&parent, &isolated).unwrap();
        assert!(!isolated.exists());
        assert_eq!(retained.parent(), Some(parent.as_path()));
        assert!(capacity::summarize(&retained).unwrap().protected);
        assert!(inspect(&retained).is_ok());
        assert!(inspect(&newest).is_ok());
        let inventory = capacity::list(&parent).unwrap();
        assert!(inventory.interrupted.is_empty());
        let preview = retention::preview(&inventory, 1, &BTreeSet::new()).unwrap();
        assert_eq!(preview.candidates, 0);
        assert!(
            preview
                .records
                .iter()
                .any(|r| r.decision == retention::Decision::Protected)
        );
        assert!(cleanup::retain_interrupted(&parent, &newest).is_err());
        create(&f.1, &isolated).unwrap();
        fs::remove_file(isolated.join("files/image.bin")).unwrap();
        assert!(cleanup::retain_interrupted(&parent, &isolated).is_err());
        assert!(isolated.join("manifest.json").exists());
        assert!(!cleanup::is_protected(&isolated).unwrap());
    }

    #[test]
    fn backup_roundtrip_preserves_notes_binary_assets_config_and_empty_directories() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        let manifest = create(&f.1, &backup).unwrap();
        assert_eq!(manifest, inspect(&backup).unwrap());
        assert_eq!(manifest.files.len(), 3);
        let restored = f.0.join("restored");
        restore(&backup, &restored, &manifest).unwrap();
        assert!(restored.join("空目录").is_dir());
        for file in manifest.files {
            assert_eq!(
                fs::read(f.1.root.join(&file.path)).unwrap(),
                fs::read(restored.join(&file.path)).unwrap()
            );
        }
    }
    #[test]
    fn backup_rejects_corruption_overwrites_and_recursive_destinations() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        let manifest = create(&f.1, &backup).unwrap();
        assert!(create(&f.1, &backup).is_err());
        assert!(create(&f.1, &f.1.root.join("nested-backup")).is_err());
        assert!(restore(&backup, &f.1.root, &manifest).is_err());
        let mut changed = manifest.clone();
        changed.source_name = "changed".into();
        assert!(restore(&backup, &f.0.join("stale"), &changed).is_err());
        fs::write(backup.join("files/image.bin"), [1, 2, 3, 4]).unwrap();
        assert!(inspect(&backup).is_err());
        assert!(restore(&backup, &f.0.join("restored"), &manifest).is_err());
        assert!(!f.0.join("restored").exists());
        assert_eq!(
            fs::read(f.1.root.join("image.bin")).unwrap(),
            [0, 255, 128, 3]
        );
    }
    #[test]
    fn backup_manifest_paths_cannot_escape_or_collide() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        let original = create(&f.1, &backup).unwrap();
        for path in ["../escaped.md", "/absolute.md", ""] {
            let mut manifest = original.clone();
            manifest.files[0].path = path.into();
            fs::write(
                backup.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            assert!(inspect(&backup).is_err());
        }
        let mut manifest = original;
        manifest.files.push(manifest.files[0].clone());
        fs::write(
            backup.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(inspect(&backup).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn backup_does_not_follow_source_or_payload_symlinks() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        create(&f.1, &backup).unwrap();
        std::os::unix::fs::symlink(&f.0, f.1.root.join("outside")).unwrap();
        assert!(create(&f.1, &f.0.join("bad-backup")).is_err());
        assert!(!f.0.join("bad-backup").exists());
        fs::remove_file(backup.join("files/image.bin")).unwrap();
        std::os::unix::fs::symlink(f.1.root.join("image.bin"), backup.join("files/image.bin"))
            .unwrap();
        assert!(inspect(&backup).is_err());
    }
}
