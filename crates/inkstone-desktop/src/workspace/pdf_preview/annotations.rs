//! Text annotations are anchored to the page's UTF-8 text layer.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::File,
    io::ErrorKind,
    ops::Range,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Style {
    Highlight,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Color {
    Yellow,
    Green,
    Pink,
}

impl Color {
    pub(super) fn rgb(self) -> u32 {
        match self {
            Self::Yellow => 0xe9b517,
            Self::Green => 0x38aa73,
            Self::Pink => 0xe96799,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Mark {
    pub page: i32,
    pub bytes: Range<usize>,
    pub quote: String,
    pub style: Style,
    pub color: Color,
    pub comment: String,
}

pub(super) struct Store(PathBuf);

impl Store {
    pub fn new(pdf: &Path) -> Self {
        let mut name = OsString::from(".inkstone-");
        name.push(pdf.file_name().unwrap_or_default());
        name.push(".annotations.json");
        Self(pdf.with_file_name(name))
    }

    pub(super) fn position(&self) -> super::Position {
        File::open(self.0.with_extension("position"))
            .ok()
            .and_then(|f| serde_json::from_reader::<_, super::Position>(f).ok())
            .filter(|p| p.page >= 0 && (0.25..=4.).contains(&p.zoom))
            .unwrap_or_default()
    }

    pub(super) fn save_position(&self, position: &super::Position) -> Result<()> {
        let mut temp = tempfile::Builder::new()
            .prefix(".inkstone-")
            .tempfile_in(self.0.parent().unwrap())?;
        serde_json::to_writer(temp.as_file_mut(), position)?;
        temp.persist(self.0.with_extension("position"))?;
        Ok(())
    }

    pub fn read(&self) -> Result<Vec<Mark>> {
        match File::open(&self.0) {
            Ok(file) => Ok(serde_json::from_reader(file)?),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn change(&self, original: Option<&Mark>, replacement: Option<Mark>) -> Result<Vec<Mark>> {
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.0.with_extension("lock"))?;
        lock.try_lock()?;
        let mut marks = self.read()?;
        if let Some(old) = original {
            let index = marks
                .iter()
                .position(|m| m == old)
                .ok_or_else(|| anyhow::anyhow!("标记已在其他窗口更改，请重新打开 PDF"))?;
            marks.remove(index);
        }
        if let Some(mark) = replacement {
            ensure!(
                !marks
                    .iter()
                    .any(|m| m.page == mark.page && m.bytes == mark.bytes && m.quote == mark.quote),
                "该选区已有标记，请从标记列表打开"
            );
            marks.push(mark);
            marks.sort_by_key(|m| (m.page, m.bytes.start, m.bytes.end));
        }
        let mut temp = tempfile::Builder::new()
            .prefix(".inkstone-")
            .tempfile_in(self.0.parent().unwrap())?;
        serde_json::to_writer(temp.as_file_mut(), &marks)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.0)?;
        Ok(marks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_changes_and_preserve_unreadable_data() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::new(&folder.path().join("论文.pdf"));
        let a = Mark {
            page: 0,
            bytes: 0..3,
            quote: "文".into(),
            style: Style::Underline,
            color: Color::Pink,
            comment: "备注".into(),
        };
        let b = Mark {
            page: 1,
            ..a.clone()
        };
        store.change(None, Some(a.clone())).unwrap();
        store.change(None, Some(b.clone())).unwrap();
        store.change(Some(&a), None).unwrap();
        assert_eq!(store.read().unwrap(), vec![b]);
        assert!(store.change(Some(&a), None).is_err());
        std::fs::write(&store.0, "{").unwrap();
        assert!(store.change(None, Some(a)).is_err());
        assert_eq!(std::fs::read(&store.0).unwrap(), b"{");
    }
}
