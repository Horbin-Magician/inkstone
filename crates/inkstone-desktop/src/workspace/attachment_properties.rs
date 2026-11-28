//! Editable metadata for local preview tabs, stored beside the attachment.
use super::{Workspace, pdf_preview::Paper};
use gpui::Context;
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(super) struct AttachmentProperties {
    file: PathBuf,
    original: Option<(u64, SystemTime)>,
    extracted: String,
    pub status: Option<String>,
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
            extracted: String::new(),
            status: None,
            source,
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.source
            .as_ref()
            .ok()
            .map(|v| v.as_deref().unwrap_or(&self.extracted))
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
        self.status = None;
        Ok(())
    }
}

impl Workspace {
    pub(super) fn receive_paper_properties(
        &mut self,
        id: usize,
        paper: &Paper,
        cx: &mut Context<Self>,
    ) {
        let Some(properties) = self.unsaved_attachment_properties(id) else {
            return;
        };
        properties.extracted = paper.source();
        let Some(doi) = paper
            .0
            .get("doi")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            cx.notify();
            return;
        };
        properties.status = Some("正在通过 DOI 从 Crossref 获取文献信息…".into());
        let paper = paper.clone();
        let task = cx
            .background_executor()
            .spawn(async move { paper.lookup(&doi) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(properties) = this.unsaved_attachment_properties(id) else {
                    return;
                };
                match result {
                    Ok(paper) => {
                        properties.extracted = paper.source();
                        if let Err(e) = properties.save(properties.extracted.clone()) {
                            properties.status = Some(format!("文献信息已提取，保存失败：{e}"));
                        }
                    }
                    Err(_) => {
                        properties.status = Some("未能从 Crossref 补全，已显示本地属性。".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn unsaved_attachment_properties(&mut self, id: usize) -> Option<&mut AttachmentProperties> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == id)?
            .attachment_properties
            .as_mut()
            .filter(|p| matches!(p.source, Ok(None)))
    }
}
