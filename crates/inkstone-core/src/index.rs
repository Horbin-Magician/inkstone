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
    collections::{BTreeMap, HashMap},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct IndexedNote {
    pub text: String,
    pub parsed: ParsedNote,
    pub times: crate::file_order::FileTimes,
}
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub notes: BTreeMap<PathBuf, Arc<IndexedNote>>,
    pub files: Vec<PathBuf>,
    /// Files excluded from searchable content, with actionable read diagnostics.
    pub errors: BTreeMap<PathBuf, String>,
    /// Lowercased vault-relative path to the stored note path.
    by_path: HashMap<String, PathBuf>,
    /// Lowercased filename stem to every note using it.
    by_stem: HashMap<String, Vec<PathBuf>>,
    /// Lowercased alias to every note declaring it.
    by_alias: HashMap<String, Vec<PathBuf>>,
    /// Resolved note to the notes that link to it.
    backlinks: HashMap<PathBuf, Vec<PathBuf>>,
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
pub(crate) fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}
fn stem_key(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
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
    /// Move or drop indexed paths without rereading notes that stayed in place.
    /// `new == None` removes the note or folder, as a trash operation does.
    pub fn relocate(&self, old: &Path, new: Option<&Path>, folder: bool) -> Self {
        let map_path = |path: &Path| -> Option<PathBuf> {
            if folder && (path == old || path.starts_with(old)) {
                let suffix = path.strip_prefix(old).ok()?;
                Some(new?.join(suffix))
            } else if !folder && path == old {
                new.map(Path::to_path_buf)
            } else {
                Some(path.to_path_buf())
            }
        };
        let mut relocated = Self {
            notes: self
                .notes
                .iter()
                .filter_map(|(path, note)| map_path(path).map(|path| (path, note.clone())))
                .collect(),
            files: {
                let mut files: Vec<_> = self
                    .files
                    .iter()
                    .filter_map(|path| map_path(path))
                    .collect();
                files.sort();
                files
            },
            errors: self
                .errors
                .iter()
                .filter_map(|(path, error)| map_path(path).map(|path| (path, error.clone())))
                .collect(),
            ..Self::default()
        };
        relocated.rebuild_lookup();
        relocated
    }
    pub fn update(&mut self, path: PathBuf, text: String) {
        self.replace_note(path, text);
        self.reindex_backlinks();
    }
    fn replace_note(&mut self, path: PathBuf, text: String) {
        self.errors.remove(&path);
        let parsed = parse(&text);
        let times = self.notes.get(&path).map(|n| n.times).unwrap_or_default();
        self.remove_note(&path);
        self.insert_note(
            path,
            Arc::new(IndexedNote {
                text,
                parsed,
                times,
            }),
        );
    }
    fn insert_note(&mut self, path: PathBuf, note: Arc<IndexedNote>) {
        self.by_path.insert(key(&path), path.clone());
        self.by_stem
            .entry(stem_key(&path))
            .or_default()
            .push(path.clone());
        for alias in &note.parsed.aliases {
            self.by_alias
                .entry(alias.to_lowercase())
                .or_default()
                .push(path.clone());
        }
        self.notes.insert(path, note);
    }
    fn remove_note(&mut self, path: &Path) {
        let Some(note) = self.notes.remove(path) else {
            return;
        };
        self.by_path.remove(&key(path));
        if let Some(paths) = self.by_stem.get_mut(&stem_key(path)) {
            paths.retain(|candidate| candidate != path);
        }
        for alias in &note.parsed.aliases {
            if let Some(paths) = self.by_alias.get_mut(&alias.to_lowercase()) {
                paths.retain(|candidate| candidate != path);
            }
        }
    }
    /// A new or removed stem/alias can change every shortest link, so rebuild
    /// once from the lookup maps instead of scanning the vault per link.
    fn reindex_backlinks(&mut self) {
        let mut backlinks: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let paths: Vec<_> = self.notes.keys().cloned().collect();
        for from in paths {
            for target in self.outgoing(&from) {
                backlinks.entry(target).or_default().push(from.clone());
            }
        }
        for sources in backlinks.values_mut() {
            sources.sort();
            sources.dedup();
        }
        self.backlinks = backlinks;
    }
    fn outgoing(&self, from: &Path) -> Vec<PathBuf> {
        let Some(note) = self.notes.get(from) else {
            return vec![];
        };
        let mut targets = Vec::new();
        for link in &note.parsed.links {
            if let Resolution::Found(path) = self.resolve(from, &link.target) {
                targets.push(path);
            }
        }
        for (url, _) in &note.parsed.standard_links {
            if let Resolution::Found(path) = self.resolve_markdown(from, url).0 {
                targets.push(path);
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }
    fn rebuild_lookup(&mut self) {
        self.by_path.clear();
        self.by_stem.clear();
        self.by_alias.clear();
        let notes = std::mem::take(&mut self.notes);
        for (path, note) in notes {
            self.insert_note(path, note);
        }
        self.reindex_backlinks();
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
        let mut links_changed = false;
        for path in paths {
            match vault.read(&path) {
                Ok(Some(text)) => {
                    self.errors.remove(&path);
                    if self.notes.get(&path).is_none_or(|n| n.text != text) {
                        self.replace_note(path.clone(), text);
                        links_changed = true;
                    }
                    self.refresh_file_times(vault, &path);
                }
                Ok(None) => {
                    links_changed |= self.notes.contains_key(&path);
                    self.remove_note(&path);
                    self.errors.remove(&path);
                }
                Err(error) => {
                    links_changed |= self.notes.contains_key(&path);
                    self.remove_note(&path);
                    self.errors.insert(path, error.to_string());
                }
            }
        }
        if links_changed {
            self.reindex_backlinks();
        }
        Ok(())
    }
}
