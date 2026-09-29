use super::*;

impl Workspace {
    pub(super) fn open_graph(&mut self, local: bool, window: &mut Window, cx: &mut Context<Self>) {
        let root = if local {
            self.active
                .and_then(|i| self.tabs.get(i))
                .map(|t| t.path.clone())
                .filter(|p| !p.as_os_str().is_empty())
        } else {
            None
        };
        if local && root.is_none() {
            self.status = "请先打开一篇笔记。".into();
            cx.notify();
            return;
        }
        let graph = cx.new(|cx| {
            crate::graph_view::GraphView::new(
                self.index.clone(),
                root,
                self.ui.prefs.light,
                if local {
                    self.ui.prefs.local_graph.clone()
                } else {
                    self.ui.prefs.graph.clone()
                },
                window,
                cx,
            )
        });
        self.graph_subscription =
            Some(
                cx.subscribe_in(&graph, window, |this, _, event, window, cx| {
                    match event {
                        crate::graph_view::GraphEvent::Settings(options) => {
                            if options.root.is_some() {
                                this.ui.prefs.local_graph = options.clone();
                            } else {
                                this.ui.prefs.graph = options.clone();
                            }
                            this.persist_workspace(cx);
                        }
                        crate::graph_view::GraphEvent::Open(path, missing) => {
                            this.graph_open = false;
                            if *missing && !this.tabs.iter().any(|t| t.path == *path) {
                                this.add_tab(path.clone(), None, true, window, cx);
                                this.save_all(window, cx);
                            } else {
                                this.open_note(path.clone(), window, cx);
                            }
                        }
                        crate::graph_view::GraphEvent::Close => {
                            this.graph_open = false;
                            if let Some(p) = this.current_pane() {
                                p.update(cx, |p, cx| p.focus_view(window, cx));
                            }
                        }
                    }
                    cx.notify();
                }),
            );
        self.graph = Some(graph);
        self.graph_open = true;
        window.focus(&self.ui.workspace_focus, cx);
        cx.notify();
    }
    pub(super) fn duplicate_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        self.sync_from_split(window, cx);
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        if self.has_pending_input(tab.id, window, cx) {
            self.status = "请完成输入法组词后复制笔记。".into();
            cx.notify();
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let source = tab.path.clone();
        if source.as_os_str().is_empty() {
            return;
        }
        let text = tab.pane.read(cx).editor.read(cx).value().to_string();
        let reserved = self.tabs.iter().map(|t| t.path.clone()).collect::<Vec<_>>();
        let generation = self.generation;
        self.ui.pending_file_writes += 1;
        let task = cx
            .background_executor()
            .spawn(async move { vault.duplicate_note(&source, &text, &reserved) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((path, receipt)) => {
                        this.status = format!("已创建副本：{}", path.display());
                        this.add_tab(path, Some(receipt.text), false, window, cx);
                        this.rescan = true;
                        this.refresh_requested = true;
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn sync_reference_contexts(&mut self, cx: &mut Context<Self>) {
        let root = self
            .vault
            .as_ref()
            .map(|v| v.root.clone())
            .unwrap_or_default();
        for tab in &self.tabs {
            tab.pane.update(cx, |p, cx| {
                p.set_reference_context(tab.path.clone(), root.clone(), self.index.clone(), cx)
            });
        }
        if let Some(split) = &self.views.split
            && let Some(tab) = self.tabs.iter().find(|t| t.id == split.source)
        {
            split.pane.update(cx, |p, cx| {
                p.set_reference_context(tab.path.clone(), root, self.index.clone(), cx)
            });
        }
    }
    pub(super) fn follow_reference(
        &mut self,
        reference: inkstone::rendering::Reference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if reference.wiki {
            let note_exists = matches!(
                self.index.resolve(&reference.from, &reference.target),
                Resolution::Found(_)
            );
            if !note_exists
                && let Some(vault) = &self.vault
                && let Some(path) =
                    inkstone::rendering::asset_path(&vault.root, &reference, &self.index.files)
            {
                cx.open_with_system(&path);
                return;
            }
            if !note_exists && inkstone::rendering::attachment_target(&reference.target) {
                self.status = format!("附件不存在：{}", reference.target);
                cx.notify();
                return;
            }
            self.follow_link(reference.from, reference.target, window, cx);
        } else {
            self.follow_markdown_link(reference.from, &reference.target, window, cx);
        }
    }
    pub(super) fn toggle_referenced_task(
        &mut self,
        target: inkstone::rendering::TaskTarget,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.file_operation {
            return;
        }
        let Some(after) =
            inkstone::index::set_task(&target.baseline, target.marker.clone(), checked)
        else {
            return;
        };
        if let Some(id) = self
            .tabs
            .iter()
            .find(|t| t.path == target.path)
            .map(|t| t.id)
            && self.has_pending_input(id, window, cx)
        {
            self.status = "请完成输入法组词后更改任务。".into();
            cx.notify();
            return;
        }
        if let Some(tab) = self.tabs.iter().find(|t| t.path == target.path) {
            let editor = tab.pane.read(cx).editor.clone();
            if editor.read(cx).value().as_ref() != target.baseline.as_ref() {
                self.status = "任务内容已改变，请等待预览更新后重试。".into();
                cx.notify();
                return;
            }
            editor.update(cx, |s, cx| {
                let selected = s.selected_range();
                let scroll = s.scroll_offset();
                s.set_selected_range(target.marker, cx);
                s.replace(if checked { "x" } else { " " }, window, cx);
                s.set_selected_range(selected, cx);
                s.set_scroll_offset(scroll, cx);
            });
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        let previous = self.index.clone();
        let work_index = previous.clone();
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            let receipt = vault.save(&target.path, Some(&target.baseline), &after)?;
            let mut index = (*work_index).clone();
            index.update(target.path, receipt.text);
            Ok::<_, VaultError>(Arc::new(index))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(index) => {
                        if Arc::ptr_eq(&this.index, &previous) {
                            this.index = index;
                            this.sync_index_ui(cx);
                            this.run_search(cx);
                        }
                        this.status = "任务状态已保存".into();
                    }
                    Err(error) => {
                        this.ui.window_close_requested = false;
                        this.status = error.to_string();
                    }
                }
                this.refresh_requested = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn manage_folder(
        &mut self,
        old: PathBuf,
        new: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if self.ui.pending_file_writes > 0 || self.ui.link_update.is_some() {
            return;
        }
        if self.tabs.iter().any(|t| t.dirty || t.saving || t.conflict) {
            self.status = "请先保存打开的笔记，再管理文件夹。".into();
            self.save_all(window, cx);
            return;
        }
        let ids: Vec<_> = self.tabs.iter().map(|t| t.id).collect();
        if ids
            .into_iter()
            .any(|id| self.has_pending_input(id, window, cx))
        {
            self.status = "请完成输入法组词后管理文件夹。".into();
            cx.notify();
            return;
        }
        let generation = self.generation;
        let from = old.clone();
        let to = new.clone();
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            if let Some(to) = to {
                let index = Index::build(&vault)?;
                let edits = index.relocation_edits(&from, &to, true, Some(&vault.root));
                vault.rename_folder(&from, &to)?;
                Ok(edits)
            } else {
                vault.trash_folder(&from).map(|_| vec![])
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, w, cx| {
                if this.generation != generation {
                    return;
                }
                this.ui.file_operation = false;
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                match result {
                    Ok(edits) => {
                        if let Some(new) = &new {
                            this.ui.prefs.locations.relocate(&old, new);
                        }
                        let active = this.active.and_then(|i| this.tabs.get(i)).map(|t| t.id);
                        this.tabs.retain_mut(|tab| {
                            if let Ok(suffix) = tab.path.strip_prefix(&old) {
                                if let Some(new) = &new {
                                    tab.path = new.join(suffix);
                                } else if tab.dirty {
                                    tab.conflict = true;
                                    return true;
                                } else {
                                    return false;
                                }
                            }
                            true
                        });
                        for paths in [
                            &mut this.ui.prefs.bookmarks,
                            &mut this.ui.prefs.pinned_paths,
                            &mut this.ui.prefs.expanded_folders,
                            &mut this.ui.history.entries,
                        ] {
                            paths.retain_mut(|path| {
                                if let Ok(suffix) = path.strip_prefix(&old) {
                                    if let Some(new) = &new {
                                        *path = new.join(suffix);
                                    } else {
                                        return false;
                                    }
                                }
                                true
                            });
                        }
                        this.ui.closed.retain_mut(|closed| {
                            if let Ok(suffix) = closed.view.path.strip_prefix(&old) {
                                if let Some(new) = &new {
                                    closed.view.path = new.join(suffix);
                                } else {
                                    return false;
                                }
                            }
                            true
                        });
                        this.ui.history.cursor = this
                            .ui
                            .history
                            .cursor
                            .min(this.ui.history.entries.len().saturating_sub(1));
                        this.active = active
                            .and_then(|id| this.tabs.iter().position(|t| t.id == id))
                            .or_else(|| (!this.tabs.is_empty()).then_some(0));
                        this.status = if new.is_some() {
                            "文件夹已移动。".into()
                        } else {
                            "文件夹已移入可恢复回收站。".into()
                        };
                        this.offer_link_updates(edits, w, cx);
                        this.sync_reference_contexts(cx);
                        this.rescan = true;
                        this.refresh_requested = true;
                        this.tick(w, cx);
                    }
                    Err(e) => this.status = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn edit_property(
        &mut self,
        name: &str,
        value: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_active_note(w, cx) {
            return;
        }
        self.ui.property_open = true;
        let display = serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_string());
        self.ui
            .property_key
            .update(cx, |s, cx| s.set_value(name.to_string(), w, cx));
        self.ui.property_value.update(cx, |s, cx| {
            s.set_value(display, w, cx);
            s.focus(w, cx);
        });
        cx.notify();
    }
    pub(super) fn save_property(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.current_pane() else {
            return;
        };
        let text = pane.read(cx).editor.read(cx).value();
        match inkstone::properties::set(
            &text,
            &self.ui.property_key.read(cx).value(),
            &self.ui.property_value.read(cx).value(),
        ) {
            Ok(text) => {
                pane.read(cx).editor.clone().update(cx, |s, cx| {
                    s.replace_all(text, w, cx);
                    s.focus(w, cx);
                });
                self.ui.property_open = false;
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    pub(super) fn open_daily(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() {
            self.choose_vault(window, cx);
            return;
        }
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let path = PathBuf::from(format!("日记/{date}.md"));
        if self.files.contains(&path) || self.tabs.iter().any(|t| t.path == path) {
            self.open_note(path, window, cx);
        } else {
            self.add_tab(path, None, true, window, cx);
            self.insert_text(&format!("# {date}\n\n"), window, cx);
            self.save_all(window, cx);
        }
    }
    pub(super) fn insert_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ensure_active_note(window, cx) {
            return;
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |pane, cx| {
                pane.reading = false;
                pane.editor.update(cx, |s, cx| {
                    s.replace(text.to_string(), window, cx);
                    s.focus(window, cx);
                });
                cx.notify();
            });
        }
    }
    pub(super) fn insert_template(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(note) = self.index.notes.get(path) else {
            return;
        };
        let now = chrono::Local::now();
        let title = self
            .active
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.path.file_stem())
            .unwrap_or_default()
            .to_string_lossy();
        let text = note
            .text
            .replace("{{date}}", &now.format("%Y-%m-%d").to_string())
            .replace("{{time}}", &now.format("%H:%M").to_string())
            .replace("{{title}}", &title);
        self.ui.quick_open = false;
        self.ui.template_mode = false;
        self.insert_text(&text, window, cx);
        cx.notify();
    }
    pub(super) fn choose_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_none() {
            self.status = "请先打开要插入附件的笔记。".into();
            cx.notify();
            return;
        }
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("插入附件".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = dialog.await {
                let _ = this.update_in(cx, |this, w, cx| this.import_files(paths, w, cx));
            }
        })
        .detach();
    }
    pub(super) fn import_files(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.import_payload(paths, None, window, cx);
    }
    pub(super) fn paste_image(
        &mut self,
        name: String,
        bytes: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.import_payload(vec![], Some((name, bytes)), window, cx);
    }
    fn import_payload(
        &mut self,
        paths: Vec<PathBuf>,
        image: Option<(String, Vec<u8>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.link_update.is_some() {
            return;
        }
        if self.graph_open {
            self.status = "请先返回笔记，再插入附件。".into();
            cx.notify();
            return;
        }
        if self.ui.file_operation {
            return;
        }
        let (Some(vault), Some(tab)) = (
            self.vault.clone(),
            self.active.and_then(|i| self.tabs.get(i)),
        ) else {
            return;
        };
        if tab.path.as_os_str().is_empty() || self.has_pending_input(tab.id, window, cx) {
            self.status = "请先打开笔记并完成输入法组词，再插入附件。".into();
            cx.notify();
            return;
        }
        let folder = match self.ui.prefs.locations.directory(Some(&tab.path), true) {
            Ok(folder) => folder,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        let id = tab.id;
        let secondary = self.views.secondary_focused;
        let generation = self.generation;
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            let mut results = vec![];
            for path in paths {
                results.push(vault.import_attachment_to(&folder, &path));
            }
            if let Some((name, bytes)) = image {
                results.push(vault.store_attachment_to(&folder, &name, &bytes));
            }
            (results, vault.scan_files())
        });
        cx.spawn_in(window, async move |this, cx| {
            let (results, files) = task.await;
            loop {
                let pending = this
                    .update_in(cx, |this, w, cx| {
                        this.generation == generation
                            && this.tabs.iter().any(|t| t.id == id)
                            && this.has_pending_input(id, w, cx)
                    })
                    .unwrap_or(false);
                if !pending {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            let _ = this.update_in(cx, |this, w, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                let catalog_error = files.as_ref().err().map(ToString::to_string);
                if let Ok(files) = files {
                    Arc::make_mut(&mut this.index).files = files;
                }
                let Some(tab) = this.tabs.iter().find(|t| t.id == id) else {
                    this.status = "附件已导入，可从笔记库的附件文件夹查看。".into();
                    cx.notify();
                    return;
                };
                let mut markdown = String::new();
                let mut errors: Vec<String> = catalog_error.clone().into_iter().collect();
                for result in results {
                    match result {
                        Ok(path) => {
                            let image = path.extension().is_some_and(|s| {
                                matches!(
                                    s.to_string_lossy().to_ascii_lowercase().as_str(),
                                    "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp"
                                )
                            });
                            markdown.push_str(&inkstone::locations::attachment_link(
                                &tab.path,
                                &path,
                                &this.index.files,
                                if catalog_error.is_some() {
                                    inkstone::locations::LinkFormat::Absolute
                                } else {
                                    this.ui.prefs.link_format
                                },
                                this.ui.prefs.use_markdown_links,
                                image,
                            ));
                            markdown.push('\n');
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                if !markdown.is_empty() {
                    let pane = if secondary {
                        this.views
                            .split
                            .as_ref()
                            .filter(|s| s.source == id)
                            .map(|s| s.pane.clone())
                            .unwrap_or_else(|| tab.pane.clone())
                    } else {
                        tab.pane.clone()
                    };
                    pane.update(cx, |pane, cx| {
                        pane.reading = false;
                        pane.editor.update(cx, |s, cx| s.replace(markdown, w, cx));
                        cx.notify();
                    });
                }
                this.status = if errors.is_empty() {
                    "附件已插入".into()
                } else {
                    errors.join("；")
                };
                this.refresh_requested = true;
                this.rescan = true;
                this.sync_reference_contexts(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
