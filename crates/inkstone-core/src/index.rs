mod links;
mod parsing;
mod search;
#[cfg(test)]
mod tests;

pub use parsing::{
    BlockReference, FoldKind, Heading, ParsedNote, TaskItem, WikiLink, anchor_range, parse,
    parse_snapshot, set_task,
};
pub(crate) use parsing::{task_box_marker, task_marker};

mod cache;
use crate::vault::{Vault, VaultError};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct IndexedNote {
    pub text: String,
    pub parsed: ParsedNote,
    pub times: crate::file_order::FileTimes,
}
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub notes: BTreeMap<PathBuf, std::sync::Arc<IndexedNote>>,
    pub files: Vec<PathBuf>,
    /// Files excluded from searchable content, with actionable read diagnostics.
    pub errors: BTreeMap<PathBuf, String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Found(PathBuf),
    Missing(PathBuf),
    Ambiguous(Vec<PathBuf>),
    Invalid,
}
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub path: PathBuf,
    pub display_name: Option<String>,
    pub offset: usize,
    pub line: usize,
    pub excerpt: String,
    pub highlights: Vec<Range<usize>>,
    pub title_highlights: Vec<Range<usize>>,
}
fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}
impl Index {
    pub fn anchor_range(&self, path: &Path, fragment: &str) -> Option<Range<usize>> {
        let note = self.notes.get(path)?;
        anchor_range(&note.text, &note.parsed, fragment)
    }
    pub fn build(vault: &Vault) -> Result<Self, VaultError> {
        let mut index = Self {
            files: vault.scan_files()?,
            ..Default::default()
        };
        let paths: Vec<_> = index
            .files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .cloned()
            .collect();
        index.refresh_paths(vault, paths)?;
        Ok(index)
    }
    pub fn note_paths(&self) -> Vec<PathBuf> {
        self.notes
            .keys()
            .chain(self.errors.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn update(&mut self, path: PathBuf, text: String) {
        self.errors.remove(&path);
        let parsed = parse(&text);
        let times = self.notes.get(&path).map(|n| n.times).unwrap_or_default();
        self.notes.insert(
            path,
            std::sync::Arc::new(IndexedNote {
                text,
                parsed,
                times,
            }),
        );
    }
    fn refresh_file_times(&mut self, vault: &Vault, path: &Path) {
        let times = vault
            .path(path)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| crate::file_order::FileTimes {
                modified: m.modified().ok(),
                created: m.created().ok(),
            })
            .unwrap_or_default();
        if let Some(note) = self.notes.get_mut(path)
            && note.times != times
        {
            std::sync::Arc::make_mut(note).times = times;
        }
    }
    pub fn refresh_paths(
        &mut self,
        vault: &Vault,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<(), VaultError> {
        for path in paths {
            match vault.read(&path) {
                Ok(Some(text)) => {
                    self.errors.remove(&path);
                    if self.notes.get(&path).is_none_or(|n| n.text != text) {
                        self.update(path.clone(), text);
                    }
                    self.refresh_file_times(vault, &path);
                }
                Ok(None) => {
                    self.notes.remove(&path);
                    self.errors.remove(&path);
                }
                Err(error) => {
                    self.notes.remove(&path);
                    self.errors.insert(path, error.to_string());
                }
            }
        }
        Ok(())
    }
}
