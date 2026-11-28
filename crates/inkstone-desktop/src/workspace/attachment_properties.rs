//! Editable metadata for local preview tabs, stored beside the attachment.
use std::{
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

pub(super) struct AttachmentProperties {
    path: PathBuf,
    pub source: Result<Option<String>, String>,
}

impl AttachmentProperties {
    pub fn open(file: &Path) -> Self {
        let mut name = OsString::from(".inkstone-");
        name.push(file.file_name().unwrap_or_default());
        name.push(".properties.yaml");
        let path = file.with_file_name(name);
        let source = Self::read(&path);
        Self { path, source }
    }

    fn read(path: &Path) -> Result<Option<String>, String> {
        match fs::read_to_string(path) {
            Ok(text) if inkstone_core::properties::block(&text).is_some() => Ok(Some(text)),
            Ok(_) => Err("附件属性文件格式无效。".into()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&mut self, source: String) -> anyhow::Result<()> {
        let lock = fs::File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))?;
        lock.try_lock()?;
        let previous = self.source.clone();
        self.source = Self::read(&self.path);
        self.source.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(
            self.source == previous,
            "附件属性已变更，请重新打开属性编辑。"
        );
        let mut temp = tempfile::Builder::new()
            .prefix(".inkstone-")
            .tempfile_in(self.path.parent().unwrap())?;
        temp.write_all(source.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)?;
        self.source = Ok(Some(source));
        Ok(())
    }
}
