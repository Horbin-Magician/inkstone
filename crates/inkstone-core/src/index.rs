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
    by_path: Arc<HashMap<String, PathBuf>>,
    /// Lowercased filename stem to every note using it.
    by_stem: Arc<HashMap<String, Vec<PathBuf>>>,
    /// Lowercased alias to every note declaring it.
    by_alias: Arc<HashMap<String, Vec<PathBuf>>>,
    /// Resolved note to the notes that link to it.
    backlinks: Arc<HashMap<PathBuf, Vec<PathBuf>>>,
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
    pub(crate) fn has_unique_stem(&self, path: &Path) -> bool {
        self.by_stem
            .get(&stem_key(path))
            .is_some_and(|paths| paths.len() == 1)
    }
    pub fn anchor_range(&self, path: &Path, fragment: &str) -> Option<Range<usize>> {
        let note = self.notes.get(path)?;
        anchor_range(&note.text, &note.parsed, fragment)
    }
    pub fn build(vault: &Vault) -> Result<Self, VaultError> {
        Self::build_from_files(vault, vault.scan_files()?)
    }
    /// Verify current disk contents while reusing unchanged parsed notes.
    /// Used before moves so external edits are included in link planning even
    /// when their watcher events have not yet reached the workspace.
    pub fn refresh_from_disk(&self, vault: &Vault) -> Result<Self, VaultError> {
        self.refresh_from_files(vault, vault.scan_files()?)
    }
    /// Reuse a caller's directory listing, including non-note assets.
    pub fn refresh_from_files(
        &self,
        vault: &Vault,
        files: Vec<PathBuf>,
    ) -> Result<Self, VaultError> {
        let mut index = Self {
            files,
            ..Default::default()
        };
        for path in &index.files {
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                continue;
            }
            match vault.read_indexed(path) {
                Ok(Some((text, times))) => {
                    let note = match self.notes.get(path) {
                        Some(note) if note.text == text && note.times == times => note.clone(),
                        Some(note) if note.text == text => Arc::new(IndexedNote {
                            text,
                            parsed: note.parsed.clone(),
                            times,
                        }),
                        _ => Arc::new(IndexedNote {
                            parsed: parse(&text),
                            text,
                            times,
                        }),
                    };
                    index.notes.insert(path.clone(), note);
                }
                Ok(None) => (),
                Err(error) => {
                    index.errors.insert(path.clone(), error.to_string());
                }
            }
        }
        index.rebuild_lookup();
        Ok(index)
    }
    pub fn build_from_files(vault: &Vault, files: Vec<PathBuf>) -> Result<Self, VaultError> {
        let mut index = Self {
            files,
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
        if self.notes.get(&path).is_some_and(|note| note.text == text) {
            return;
        }
        if self.replace_note(path, text) {
            self.reindex_backlinks();
        }
    }
    /// Index bytes the caller just wrote. A missing file is removed; existing
    /// files are not reread, because those bytes are already known.
    pub fn apply_known(
        &mut self,
        vault: &Vault,
        known: impl IntoIterator<Item = (PathBuf, String)>,
    ) -> bool {
        let mut rebuild_links = false;
        let mut changed = false;
        for (path, text) in known {
            let metadata = vault
                .path(&path)
                .ok()
                .and_then(|absolute| std::fs::metadata(absolute).ok())
                .filter(|meta| meta.is_file());
            let Some(metadata) = metadata else {
                rebuild_links |= self.notes.contains_key(&path);
                changed |= self.notes.contains_key(&path);
                self.remove_note(&path);
                changed |= self.errors.remove(&path).is_some();
                continue;
            };
            changed |= self.errors.remove(&path).is_some();
            if self.notes.get(&path).is_none_or(|note| note.text != text) {
                rebuild_links |= self.replace_note(path.clone(), text);
                changed = true;
            }
            changed |= self.set_file_times(&path, (&metadata).into());
        }
        if rebuild_links {
            self.reindex_backlinks();
        }
        changed
    }
    /// Returns whether changed names can affect resolution across the vault.
    /// Ordinary edits preserve name maps; only changed outgoing edges need work.
    fn replace_note(&mut self, path: PathBuf, text: String) -> bool {
        self.errors.remove(&path);
        let parsed = parse(&text);
        let previous = self.notes.get(&path);
        let times = previous.map(|n| n.times).unwrap_or_default();
        let names_changed = previous.is_none_or(|n| n.parsed.aliases != parsed.aliases);
        let links_changed = previous.is_none_or(|n| {
            !n.parsed
                .links
                .iter()
                .map(|l| &l.target)
                .eq(parsed.links.iter().map(|l| &l.target))
                || !n
                    .parsed
                    .standard_links
                    .iter()
                    .map(|(url, _)| url)
                    .eq(parsed.standard_links.iter().map(|(url, _)| url))
        });
        let old_targets = if !names_changed && links_changed {
            self.outgoing(&path)
        } else {
            Vec::new()
        };
        let note = Arc::new(IndexedNote {
            text,
            parsed,
            times,
        });
        if names_changed {
            self.remove_note(&path);
            self.insert_note(path, note);
        } else {
            self.notes.insert(path.clone(), note);
            if links_changed {
                let new_targets = self.outgoing(&path);
                if old_targets != new_targets {
                    let backlinks = Arc::make_mut(&mut self.backlinks);
                    for target in &old_targets {
                        if new_targets.binary_search(target).is_ok() {
                            continue;
                        }
                        if let Some(sources) = backlinks.get_mut(target) {
                            if let Ok(position) = sources.binary_search(&path) {
                                sources.remove(position);
                            }
                            if sources.is_empty() {
                                backlinks.remove(target);
                            }
                        }
                    }
                    for target in new_targets {
                        if old_targets.binary_search(&target).is_ok() {
                            continue;
                        }
                        let sources = backlinks.entry(target).or_default();
                        if let Err(position) = sources.binary_search(&path) {
                            sources.insert(position, path.clone());
                        }
                    }
                }
            }
        }
        names_changed
    }
    fn insert_note(&mut self, path: PathBuf, note: Arc<IndexedNote>) {
        Arc::make_mut(&mut self.by_path).insert(key(&path), path.clone());
        Arc::make_mut(&mut self.by_stem)
            .entry(stem_key(&path))
            .or_default()
            .push(path.clone());
        for alias in &note.parsed.aliases {
            Arc::make_mut(&mut self.by_alias)
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
        Arc::make_mut(&mut self.by_path).remove(&key(path));
        fn remove_entry(map: &mut HashMap<String, Vec<PathBuf>>, key: String, path: &Path) {
            if let Some(paths) = map.get_mut(&key) {
                paths.retain(|candidate| candidate != path);
                if paths.is_empty() {
                    map.remove(&key);
                }
            }
        }
        remove_entry(Arc::make_mut(&mut self.by_stem), stem_key(path), path);
        for alias in &note.parsed.aliases {
            remove_entry(
                Arc::make_mut(&mut self.by_alias),
                alias.to_lowercase(),
                path,
            );
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
        self.backlinks = Arc::new(backlinks);
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
        self.by_path = Default::default();
        self.by_stem = Default::default();
        self.by_alias = Default::default();
        let notes = std::mem::take(&mut self.notes);
        for (path, note) in notes {
            self.insert_note(path, note);
        }
        self.reindex_backlinks();
    }
    fn set_file_times(&mut self, path: &Path, times: crate::file_order::FileTimes) -> bool {
        if let Some(note) = self.notes.get_mut(path)
            && note.times != times
        {
            std::sync::Arc::make_mut(note).times = times;
            return true;
        }
        false
    }
    pub fn refresh_paths(
        &mut self,
        vault: &Vault,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<bool, VaultError> {
        let mut rebuild_links = false;
        let mut changed = false;
        for path in paths {
            match vault.read_indexed(&path) {
                Ok(Some((text, times))) => {
                    changed |= self.errors.remove(&path).is_some();
                    if self.notes.get(&path).is_none_or(|n| n.text != text) {
                        rebuild_links |= self.replace_note(path.clone(), text);
                        changed = true;
                    }
                    changed |= self.set_file_times(&path, times);
                }
                Ok(None) => {
                    rebuild_links |= self.notes.contains_key(&path);
                    changed |= self.notes.contains_key(&path);
                    self.remove_note(&path);
                    changed |= self.errors.remove(&path).is_some();
                }
                Err(error) => {
                    rebuild_links |= self.notes.contains_key(&path);
                    changed |= self.notes.contains_key(&path);
                    self.remove_note(&path);
                    let error = error.to_string();
                    changed |= self.errors.get(&path) != Some(&error);
                    self.errors.insert(path, error);
                }
            }
        }
        if rebuild_links {
            self.reindex_backlinks();
        }
        Ok(changed)
    }
}
