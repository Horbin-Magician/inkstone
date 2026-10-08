//! Note creation, rename, trash, and draft copies.

use super::*;

impl Workspace {
    pub(super) fn restore_text_as_copy(
        &mut self,
        original: &std::path::Path,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = original.file_stem().unwrap_or_default().to_string_lossy();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = PathBuf::from(format!("{name}-恢复-{unique}.md"));
        self.add_tab(path, None, true, window, cx);
        let editor = self.tabs.last().unwrap().pane.read(cx).editor.clone();
        editor.update(cx, |state, cx| state.set_value(text, window, cx));
        self.notifications
            .publish("恢复内容已打开为新笔记，原文件与恢复记录均保留。".into());
        let id = self.tabs.last().unwrap().id;
        self.save_restored_copy(id, window, cx);
        cx.notify();
    }
    pub(super) fn manage_note(&mut self, trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active.and_then(|i| self.tabs.get(i)).map(|tab| tab.id) else {
            return;
        };
        let name = self.name.read(cx).value().trim().to_string();
        self.manage_named_note(id, trash, name, window, cx);
    }
    pub(super) fn manage_named_note(
        &mut self,
        id: usize,
        trash: bool,
        requested_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.file_writes.can_start_exclusive_operation() || self.ui.link_update.is_some() {
            return;
        }
        if self.has_pending_input(id, window, cx) {
            self.notifications
                .publish("请完成当前编辑后再更改文件路径。".into());
            cx.notify();
            return;
        }
        if trash
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path.as_os_str().is_empty())
        {
            self.close_tab(window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if !trash
            && self.tabs.iter().any(|t| {
                t.save.persistence.is_dirty()
                    || t.save.persistence.is_saving()
                    || t.save.persistence.has_conflict()
            })
        {
            self.notifications
                .publish("请先保存打开的笔记，再重命名并更新内部链接。".into());
            self.save_all(window, cx);
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        if tab.save.persistence.is_dirty()
            || tab.save.persistence.is_saving()
            || tab.save.persistence.has_conflict()
        {
            self.notifications
                .publish("请先保存并处理冲突，再重命名或移入回收区。".into());
            cx.notify();
            return;
        }
        let Some(baseline) = tab.save.persistence.baseline().clone() else {
            return;
        };
        let mut name = requested_name;
        if !trash && name.is_empty() {
            self.notifications
                .publish("请在名称框输入新文件名。".into());
            cx.notify();
            return;
        }
        if !name.to_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        let dest = PathBuf::from(name);
        let path = tab.path.clone();
        let id = tab.id;
        let document = tab.save.clone();
        let generation = self.generation;
        let Some(write_ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        if !tab.save.persistence.begin_file_operation() {
            self.file_writes.finish(write_ticket);
            return;
        }
        let previous_index = self.index.clone();
        let task = cx.background_executor().spawn(async move {
            if trash {
                vault
                    .trash_note(&path, &baseline)
                    .map(|p| (true, p, LinkEdits::new()))
            } else {
                let index = previous_index.refresh_from_disk(&vault)?;
                let edits = index.relocation_edits(&path, &dest, false, Some(&vault.root));
                vault.rename_note(&path, &dest, &baseline)?;
                Ok((false, dest, edits))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(write_ticket);
                document.persistence.finish_operation();
                if generation != this.generation {
                    return;
                }
                let Some(index) = this
                    .tabs
                    .iter()
                    .position(|t| std::rc::Rc::ptr_eq(&t.save, &document))
                else {
                    return;
                };
                match result {
                    Ok((true, path, _)) => {
                        let removed = this.tabs[index].path.clone();
                        this.notifications
                            .publish(format!("已移入可恢复回收区：{}", path.display()));
                        this.schedule_auto_sync(true);
                        if this.tabs[index].save.persistence.is_dirty() {
                            this.tabs[index].save.persistence.preserve_external_change();
                            this.notifications
                                .append("；操作期间的新编辑已保留，可另存副本。");
                        } else {
                            this.tabs
                                .retain(|tab| !std::rc::Rc::ptr_eq(&tab.save, &document));
                            this.active = if this.tabs.is_empty() {
                                None
                            } else {
                                Some(index.min(this.tabs.len() - 1))
                            };
                            this.remove_missing_views();
                        }
                        this.apply_relocated_index(&removed, None, false, cx);
                        this.persist_workspace(cx);
                    }
                    Ok((false, path, edits)) => {
                        let focus_after = this
                            .ui
                            .inline_title
                            .as_ref()
                            .is_some_and(|edit| edit.id == id && edit.focus_after);
                        if this
                            .ui
                            .inline_title
                            .as_ref()
                            .is_some_and(|edit| edit.id == id)
                        {
                            this.ui.inline_title = None;
                        }
                        let old = this.tabs[index].path.clone();
                        for item in this
                            .ui
                            .prefs
                            .bookmarks
                            .iter_mut()
                            .chain(this.ui.prefs.pinned_paths.iter_mut())
                        {
                            if *item == old {
                                *item = path.clone();
                            }
                        }
                        this.relocate_document(&document, path);
                        let renamed = this.tabs[index].path.clone();
                        this.relocate_navigation(&old, Some(&renamed), false, cx);
                        this.apply_relocated_index(&old, Some(&renamed), false, cx);
                        this.persist_workspace(cx);
                        if focus_after && let Some(pane) = this.current_pane() {
                            pane.update(cx, |p, cx| p.focus_view(window, cx));
                        }
                        this.notifications.publish("已重命名。".into());
                        this.offer_link_updates(edits, window, cx);
                    }
                    Err(error) => {
                        this.notifications.publish(error.to_string());
                        this.ui.window_close_requested = false;
                    }
                }
                this.structure_changed = true;
                this.refresh_requested = true;
                this.tick(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn create_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.as_ref() else {
            self.notifications.publish("请先打开笔记库。".into());
            cx.notify();
            return;
        };
        let mut name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            name = "未命名.md".into();
        }
        if !name.to_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        let path = PathBuf::from(name);
        if let Err(error) = vault.path(&path) {
            self.notifications.publish(error.to_string());
            cx.notify();
            return;
        }
        if self.files.contains(&path) || self.tabs.iter().any(|t| t.path == path) {
            self.notifications.publish("同名笔记已存在。".into());
            cx.notify();
            return;
        }
        self.add_tab(path, None, true, window, cx);
        self.save_all(window, cx);
    }
    pub(super) fn focus_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() {
            self.choose_vault(window, cx);
            return;
        }
        self.command_open = false;
        let current = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.as_path());
        let folder = match self.ui.prefs.locations.directory(current, false) {
            Ok(folder) => folder,
            Err(error) => {
                self.notifications.publish(error.into());
                cx.notify();
                return;
            }
        };
        self.begin_tree_name(super::ui::NameMode::New, folder, window, cx);
    }
    pub(super) fn quick_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = &self.vault else {
            self.notifications
                .publish("请先打开笔记库，再开始快速记录。".into());
            return;
        };
        let current = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.as_path());
        let folder = match self.ui.prefs.locations.directory(current, false) {
            Ok(folder) => folder,
            Err(error) => {
                self.notifications.publish(error.into());
                return;
            }
        };
        let stem = chrono::Local::now()
            .format("速记 %Y-%m-%d %H%M%S")
            .to_string();
        let mut suffix = 0;
        let path = loop {
            let name = if suffix == 0 {
                format!("{stem}.md")
            } else {
                format!("{stem}-{suffix}.md")
            };
            let path = folder.join(name);
            if !vault.root.join(&path).exists() && !self.tabs.iter().any(|t| t.path == path) {
                break path;
            }
            suffix += 1;
        };
        self.name.update(cx, |input, cx| {
            input.set_value(path.to_string_lossy().to_string(), window, cx)
        });
        self.create_note(window, cx);
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |pane, cx| {
                pane.reading = false;
                pane.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        }
    }
}
