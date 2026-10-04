//! External file refresh and index/tree synchronization.

use super::*;

impl Workspace {
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.refresh_requested = false;
        self.refreshing = true;
        let generation = self.generation;
        let folder_revision = self.folder_revision;
        let changed = std::mem::take(&mut self.changed_paths);
        let structure_changed = std::mem::take(&mut self.structure_changed);
        let rescan = std::mem::take(&mut self.rescan) || (changed.is_empty() && !structure_changed);
        let previous = self.index.clone();
        let requests: Vec<_> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.path.clone(), t.save.baseline.borrow().clone()))
            .collect();
        let task = cx.background_executor().spawn(async move {
            let index = if rescan {
                Arc::new(Index::build(&vault)?)
            } else if structure_changed {
                refresh_created_index(&vault, previous.clone(), changed)?
            } else {
                let mut index = (*previous).clone();
                index.refresh_paths(&vault, changed)?;
                Arc::new(index)
            };
            let files = index.note_paths();
            let mut documents = vec![];
            for (id, path, baseline) in requests {
                if Arc::ptr_eq(&previous, &index) {
                    break;
                }
                let disk = vault.read(&path);
                documents.push((id, path, baseline, disk));
            }
            let folders = vault.folders()?;
            Ok::<_, VaultError>((files, documents, index, folders))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation { return; }
                this.refreshing = false;
                if this.ui.file_operation {
                    this.refresh_requested = true;
                    this.rescan = true;
                    return;
                }
                match result {
                    Ok((files, documents, index, folders)) => {
                        if this.folder_revision == folder_revision {
                            this.ui.folders = folders;
                        }
                        this.files = files;
                        let index_changed = !Arc::ptr_eq(&this.index, &index);
                        if index_changed {
                            this.index = index;
                            this.sync_index_ui(cx);
                        } else if this.ui.tree_folders != this.ui.folders {
                            this.ui.tree_folders = this.ui.folders.clone();
                            this.rebuild_sorted_tree(cx);
                        }
                        for (id, path, baseline, disk) in documents {
                            let split_pending=this.has_pending_input(id,window,cx);
                            let Some(tab) = this.tabs.iter_mut().find(|t|t.id == id) else { continue; };
                            if tab.save.saving.get() || tab.path!=path || *tab.save.baseline.borrow() != baseline { this.refresh_requested = true; continue; }
                            let disk = match disk {
                                Ok(disk) => disk,
                                Err(error) => {
                                    tab.save.conflict.set(true);
                                    tab.save.dirty.set(true);
                                    this.status = format!("无法读取 {}：{error}。编辑内容已保留。", path.display());
                                    continue;
                                }
                            };
                            if disk == *tab.save.baseline.borrow() { continue; }
                            if tab.save.dirty.get() || split_pending || disk.is_none() {
                                tab.save.conflict.set(true); tab.save.dirty.set(true);
                                this.status = format!("{} 在外部发生变化。编辑内容已保留；可用“另存为副本”保存当前版本。", tab.path.display());
                            } else if let Some(text) = disk {
                                let editor = tab.save.editor.clone();
                                let selection = editor.read(cx).selected_range();
                                tab.save.baseline.replace(Some(text.clone()));
                                editor.update(cx, |state, cx| { state.set_value(text, window, cx); state.set_selected_range(selection, cx); });
                                this.status = format!("已重新加载外部修改：{}", tab.path.display());
                                this.document_changed(editor, window, cx);
                            }
                        }
                        if index_changed {
                            this.run_search(cx);
                        }
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn sync_index_ui(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.pane.update(cx, |pane, _| {
                pane.set_paths(self.link_paths_for(&tab.path));
                if let Some(vault) = &self.vault {
                    pane.image_dir = vault
                        .root
                        .join(tab.path.parent().unwrap_or(std::path::Path::new("")));
                }
            });
        }
        if let Some(split) = &self.views.split {
            if let Some(tab) = self.tabs.iter().find(|t| t.id == split.source) {
                let image_dir = tab.pane.read(cx).image_dir.clone();
                let paths = self.link_paths_for(&tab.path);
                split.pane.update(cx, |p, _| {
                    p.image_dir = image_dir;
                    p.set_paths(paths);
                });
            } else {
                self.views.split = None;
                self.views.secondary_focused = false;
            }
        }
        let files: Vec<_> = self.index.note_paths();
        if self.tree_files != files
            || self.ui.tree_folders != self.ui.folders
            || self.ui.prefs.sort_by != inkstone_core::file_order::SortBy::Name
        {
            self.ui.tree_folders = self.ui.folders.clone();
            self.tree_files = files.clone();
            self.rebuild_sorted_tree(cx);
        }
        self.sync_reference_contexts(cx);
    }
}

// Creation events (including Windows' CreateKind::Any) can describe an empty
// folder or an imported directory. Discover new files without reparsing notes
// whose contents did not change.
pub(super) fn refresh_created_index(
    vault: &Vault,
    previous: Arc<Index>,
    mut changed: std::collections::BTreeSet<PathBuf>,
) -> Result<Arc<Index>, VaultError> {
    let files = vault.scan_files()?;
    let notes: std::collections::BTreeSet<_> = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .cloned()
        .collect();
    changed.retain(|p| notes.contains(p));
    changed.extend(
        notes
            .iter()
            .filter(|p| !previous.notes.contains_key(*p))
            .cloned(),
    );
    let removed = previous
        .notes
        .keys()
        .chain(previous.errors.keys())
        .any(|p| !notes.contains(p));
    if changed.is_empty() && !removed && files == previous.files {
        return Ok(previous);
    }
    let mut index = (*previous).clone();
    index.notes.retain(|p, _| notes.contains(p));
    index.errors.retain(|p, _| notes.contains(p));
    index.refresh_paths(vault, changed)?;
    index.files = files;
    Ok(Arc::new(index))
}

pub(super) fn make_tree(files: &[PathBuf]) -> Vec<TreeItem> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut children: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for file in files {
        let mut path = PathBuf::new();
        for component in file.components() {
            let parent = path.clone();
            path.push(component);
            children.entry(parent).or_default().insert(path.clone());
        }
    }
    fn build(
        parent: &std::path::Path,
        children: &BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    ) -> Vec<TreeItem> {
        children
            .get(parent)
            .into_iter()
            .flat_map(|items| items.iter())
            .map(|path| {
                TreeItem::new(
                    path.to_string_lossy().to_string(),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                )
                .children(build(path, children))
            })
            .collect()
    }
    build(std::path::Path::new(""), &children)
}
