//! Editable metadata for local preview tabs, stored beside the attachment.
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone)]
pub(super) struct AttachmentProperties {
    file: PathBuf,
    original: Option<(u64, SystemTime)>,
    pub source: Result<Option<String>, String>,
}

impl AttachmentProperties {
    pub fn open(file: &Path) -> Self {
        let source = Self::read(&inkstone_core::properties::sidecar(file));
        Self {
            file: file.to_owned(),
            original: fs::metadata(file)
                .ok()
                .and_then(|m| Some((m.len(), m.modified().ok()?))),
            source,
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.source
            .as_ref()
            .ok()
            .map(|v| v.as_deref().unwrap_or_default())
    }

    fn read(path: &Path) -> Result<Option<String>, String> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err("附件属性路径必须是普通文件。".into()),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        }
        match fs::read_to_string(path) {
            Ok(text) if inkstone_core::properties::block(&text).is_some() => Ok(Some(text)),
            Ok(_) => Err("附件属性文件格式无效。".into()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&mut self, source: String) -> anyhow::Result<()> {
        let lock = fs::File::open(&self.file)?;
        lock.try_lock()?;
        let meta = lock.metadata()?;
        anyhow::ensure!(
            self.original == Some((meta.len(), meta.modified()?)),
            "附件已变更，请重新打开文件。"
        );
        let path = inkstone_core::properties::sidecar(&self.file);
        let previous = self.source.clone();
        self.source = Self::read(&path);
        self.source.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(
            self.source == previous,
            "附件属性已变更，请重新打开属性编辑。"
        );
        let mut temp = tempfile::Builder::new()
            .prefix(".inkstone-")
            .tempfile_in(path.parent().unwrap())?;
        temp.write_all(source.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&path)?;
        self.source = Ok(Some(source));
        Ok(())
    }
}
