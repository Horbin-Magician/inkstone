//! Versioned, checksummed directory backups. Publication/restoration never overwrites.
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
    inspect(&stage.0)?;
    move_no_replace(&stage.0, &destination)?;
    Ok(manifest)
}

pub fn inspect(source: &Path) -> io::Result<Manifest> {
    if is_reparse(&fs::symlink_metadata(source)?) {
        return Err(invalid("备份目录不能是符号链接"));
    }
    let path = safe_path(source, Path::new("manifest.json"))?;
    if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
        return Err(invalid("备份清单过大"));
    }
    let manifest: Manifest = serde_json::from_reader(fs::File::open(path)?)?;
    if manifest.version != 1 {
        return Err(invalid("不支持此备份格式版本"));
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
        if transfer(&safe_path(&payload, &entry.path)?, None)?
            != (entry.bytes, entry.sha256.clone())
        {
            return Err(invalid(format!("内容校验失败：{}", entry.path.display())));
        }
    }
    Ok(manifest)
}

pub fn restore(source: &Path, destination: &Path, expected: &Manifest) -> io::Result<Manifest> {
    let manifest = inspect(source)?;
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
