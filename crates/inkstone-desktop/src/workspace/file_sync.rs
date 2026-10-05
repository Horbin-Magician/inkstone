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
        let mut changed = std::mem::take(&mut self.changed_paths);
        let known = std::mem::take(&mut self.known_writes);
        let structure_changed = std::mem::take(&mut self.structure_changed);
        let rescan = std::mem::take(&mut self.rescan)
            || (changed.is_empty() && known.is_empty() && !structure_changed);
        // Structural refresh reads the vault. Keep known paths in that read so a
        // delete or import cannot be hidden by bytes we wrote earlier.
        let known = if structure_changed || rescan {
            changed.extend(known.into_keys());
            std::collections::BTreeMap::new()
        } else {
            known
        };
        let previous = self.index.clone();
        let folders = self.ui.folders.clone();
        let requests: Vec<_> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.path.clone(), t.save.persistence.baseline().clone()))
            .collect();
        let task = cx.background_executor().spawn(async move {
            let (index, folders) = if rescan {
                let tree = vault.scan_tree()?;
                (
                    Arc::new(Index::build_from_files(&vault, tree.files)?),
                    tree.folders,
                )
            } else if structure_changed {
                refresh_created_index(&vault, previous.clone(), changed)?
            } else {
                // Reuse the folder listing unless a changed note introduces a parent
                // that is not already visible. Index bytes we already wrote first;
                // a watcher event for the same path still reads the file, so an
                // external edit in the same burst replaces those bytes.
                let mut index = (*previous).clone();
                let folders = if changed.iter().chain(known.keys()).any(|path| {
                    path.parent().is_some_and(|parent| {
                        !parent.as_os_str().is_empty()
                            && !folders.iter().any(|folder| folder == parent)
                    })
                }) {
                    vault.folders()?
                } else {
                    folders
                };
                let known_changed = index.apply_known(&vault, known);
                let disk_changed = index.refresh_paths(&vault, changed)?;
                let index = if known_changed || disk_changed {
                    Arc::new(index)
                } else {
                    previous.clone()
                };
                (index, folders)
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
                            if tab.save.persistence.is_saving() || tab.path!=path || *tab.save.persistence.baseline() != baseline { this.refresh_requested = true; continue; }
                            let disk = match disk {
                                Ok(disk) => disk,
                                Err(error) => {
                                    tab.save.persistence.preserve_external_change();
                                    this.notifications.publish(format!("无法读取 {}：{error}。编辑内容已保留。", path.display()));
                                    continue;
                                }
                            };
                            match tab.save.persistence.external_change(disk, split_pending) {
                                super::save_state::ExternalChange::Unchanged => {}
                                super::save_state::ExternalChange::PreserveLocal => {
                                    this.notifications.publish(format!("{} 在外部发生变化。编辑内容已保留；可用“另存为副本”保存当前版本。", tab.path.display()));
                                }
                                super::save_state::ExternalChange::Reload(text) => {
                                    let editor = tab.save.editor.clone();
                                    let selection = editor.read(cx).selected_range();
                                    editor.update(cx, |state, cx| { state.set_value(text, window, cx); state.set_selected_range(selection, cx); });
                                    this.notifications.publish(format!("已重新加载外部修改：{}", tab.path.display()));
                                    this.document_changed(editor, window, cx);
                                }
                            }
                        }
                        if index_changed {
                            this.run_search(cx);
                        }
                    }
                    Err(error) => this.notifications.publish(error.to_string()),
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn sync_index_ui(&mut self, cx: &mut Context<Self>) {
        self.prepare_link_paths();
        let tab_paths: Vec<_> = self.tabs.iter().map(|tab| tab.path.clone()).collect();
        for path in tab_paths {
            let paths = self.cached_link_paths(&path);
            let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) else {
                continue;
            };
            tab.pane.update(cx, |pane, _| {
                pane.set_paths(paths);
                if let Some(vault) = &self.vault {
                    pane.image_dir = vault
                        .root
                        .join(path.parent().unwrap_or(std::path::Path::new("")));
                }
            });
        }
        let split_source = self.views.split.as_ref().map(|split| split.source);
        if let Some(source) = split_source {
            let source = self
                .tabs
                .iter()
                .find(|t| t.id == source)
                .map(|tab| (tab.path.clone(), tab.pane.read(cx).image_dir.clone()));
            if let Some((path, image_dir)) = source {
                let paths = self.cached_link_paths(&path);
                let Some(split) = &self.views.split else {
                    return;
                };
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
    /// Reflect a completed rename or trash in the open index before the watcher reconciles.
    pub(super) fn apply_relocated_index(
        &mut self,
        old: &std::path::Path,
        new: Option<&std::path::Path>,
        folder: bool,
        cx: &mut Context<Self>,
    ) {
        self.ui.prefs.anchor_bookmarks.retain_mut(|entry| {
            let affected = entry.path == old || (folder && entry.path.starts_with(old));
            if affected {
                let Some(new) = new else {
                    return false;
                };
                entry.path = if folder {
                    new.join(entry.path.strip_prefix(old).unwrap())
                } else {
                    new.to_owned()
                };
            }
            true
        });
        if folder {
            self.ui.folders = self
                .ui
                .folders
                .iter()
                .filter_map(|path| {
                    if path == old {
                        new.map(std::path::Path::to_path_buf)
                    } else if let Ok(suffix) = path.strip_prefix(old) {
                        new.map(|parent| parent.join(suffix))
                    } else {
                        Some(path.clone())
                    }
                })
                .collect();
            self.ui.folders.sort();
            self.ui.folders.dedup();
            self.folder_revision += 1;
        }
        self.index = Arc::new(self.index.relocate(old, new, folder));
        self.files = self.index.note_paths();
        self.sync_index_ui(cx);
    }
    /// Record bytes already written. Content refresh indexes them without a vault walk.
    pub(super) fn note_indexed_change(
        &mut self,
        path: PathBuf,
        text: String,
        _cx: &mut Context<Self>,
    ) {
        self.known_writes.insert(path, text);
        self.refresh_requested = true;
    }
}

// Creation events (including Windows' CreateKind::Any) can describe an empty
// folder or an imported directory. Discover new files without reparsing notes
// whose contents did not change.
pub(super) fn refresh_created_index(
    vault: &Vault,
    previous: Arc<Index>,
    mut changed: std::collections::BTreeSet<PathBuf>,
) -> Result<(Arc<Index>, Vec<PathBuf>), VaultError> {
    let tree = vault.scan_tree()?;
    let files = tree.files;
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
        return Ok((previous, tree.folders));
    }
    let mut index = (*previous).clone();
    // Let the index remove missing notes through its normal update path so
    // path/alias lookup and backlinks cannot retain deleted entries.
    changed.extend(
        previous
            .notes
            .keys()
            .chain(previous.errors.keys())
            .filter(|p| !notes.contains(*p))
            .cloned(),
    );
    index.refresh_paths(vault, changed)?;
    index.files = files;
    Ok((Arc::new(index), tree.folders))
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
