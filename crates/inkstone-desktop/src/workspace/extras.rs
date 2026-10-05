use super::*;

impl Workspace {
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
                        this.note_indexed_change(path.clone(), receipt.text.clone(), cx);
                        this.add_tab(path, Some(receipt.text), false, window, cx);
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
        reference: inkstone_core::rendering::Reference,
        new_tab: bool,
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
                    inkstone_core::rendering::asset_path(&vault.root, &reference, &self.index.files)
            {
                cx.open_with_system(&path);
                return;
            }
            if !note_exists && inkstone_core::rendering::attachment_target(&reference.target) {
                self.status = format!("附件不存在：{}", reference.target);
                cx.notify();
                return;
            }
            self.follow_link(reference.from, reference.target, new_tab, window, cx);
        } else {
            self.follow_markdown_link(reference.from, &reference.target, new_tab, window, cx);
        }
    }
    pub(super) fn toggle_referenced_task(
        &mut self,
        target: inkstone_core::rendering::TaskTarget,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.file_operation {
            return;
        }
        let Some(after) =
            inkstone_core::index::set_task(&target.baseline, target.marker.clone(), checked)
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
            let editor = tab.save.editor.clone();
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
                        this.schedule_auto_sync(true);
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
        if self.tabs.iter().any(|t| {
            t.save.persistence.dirty.get()
                || t.save.persistence.saving.get()
                || t.save.persistence.conflict.get()
        }) {
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
                                    tab.save.path.replace(tab.path.clone());
                                } else if tab.save.persistence.dirty.get() {
                                    tab.save.persistence.preserve_external_change();
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
                        this.relocate_navigation(&old, new.as_deref(), true, cx);
                        this.active = active
                            .and_then(|id| this.tabs.iter().position(|t| t.id == id))
                            .or_else(|| (!this.tabs.is_empty()).then_some(0));
                        this.remove_missing_views();
                        this.status = if new.is_some() {
                            "文件夹已移动。".into()
                        } else {
                            "文件夹已移入可恢复回收站。".into()
                        };
                        this.schedule_auto_sync(true);
                        // The move is a rename. Reuse parsed notes instead of rebuilding the vault index.
                        this.apply_relocated_index(&old, new.as_deref(), true, cx);
                        this.offer_link_updates(edits, w, cx);
                        this.structure_changed = true;
                        this.refresh_requested = true;
                        this.persist_workspace(cx);
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
        self.ui.property_error.clear();
        self.ui
            .property_list_entry
            .update(cx, |s, cx| s.set_value("", w, cx));
        self.ui.property_kind = if matches!(name, "tags" | "aliases" | "cssclasses") {
            inkstone_core::properties::Kind::List
        } else {
            self.ui
                .prefs
                .property_types
                .get(&name.to_lowercase())
                .copied()
                .unwrap_or_else(|| inkstone_core::properties::Kind::infer(value))
        };
        self.ui.property_original = (!name.is_empty()).then(|| name.to_owned());
        self.ui.property_baseline = self.active.and_then(|i| self.tabs.get(i)).and_then(|tab| {
            self.current_pane()
                .map(|pane| (tab.id, pane.read(cx).editor.read(cx).value().to_string()))
        });
        let display = self.ui.property_kind.editor_value(value);
        self.ui
            .property_key
            .update(cx, |s, cx| s.set_value(name.to_string(), w, cx));
        self.ui.property_value.update(cx, |s, cx| {
            s.set_value(display, w, cx);
            s.focus(w, cx);
        });
        self.sync_property_dates(w, cx);
        if self.effective_property_kind(cx) == inkstone_core::properties::Kind::List
            && self.property_list_values(cx).is_ok()
        {
            self.ui
                .property_list_entry
                .update(cx, |s, cx| s.focus(w, cx));
        }
        if self.effective_property_kind(cx) == inkstone_core::properties::Kind::Checkbox {
            w.focus(&self.ui.modal_focus, cx);
        }
        cx.notify();
    }
    pub(super) fn effective_property_kind(
        &self,
        cx: &Context<Self>,
    ) -> inkstone_core::properties::Kind {
        if matches!(
            self.ui.property_key.read(cx).value().as_ref(),
            "tags" | "aliases" | "cssclasses"
        ) {
            inkstone_core::properties::Kind::List
        } else {
            self.ui.property_kind
        }
    }
    pub(super) fn change_property_kind(
        &mut self,
        kind: inkstone_core::properties::Kind,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use inkstone_core::properties::Kind;
        let previous = self.effective_property_kind(cx);
        if previous == kind {
            return;
        }
        if previous == Kind::List && !self.add_property_list_item(w, cx) {
            return;
        }
        if kind == Kind::List {
            let value = self.ui.property_value.read(cx).value().to_string();
            let items = if value.is_empty() {
                vec![]
            } else {
                vec![value]
            };
            self.ui.property_value.update(cx, |s, cx| {
                s.set_value(serde_json::to_string(&items).unwrap(), w, cx)
            });
        }
        self.ui.property_kind = kind;
        self.ui.property_error.clear();
        self.sync_property_dates(w, cx);
        if kind == Kind::Checkbox {
            self.ui.property_value.update(cx, |s, cx| {
                s.set_value((s.value().as_ref() == "true").to_string(), w, cx)
            });
            w.focus(&self.ui.modal_focus, cx);
        } else if kind == Kind::List {
            self.ui
                .property_list_entry
                .update(cx, |s, cx| s.focus(w, cx));
        } else {
            self.ui.property_value.update(cx, |s, cx| s.focus(w, cx));
        }
        cx.notify();
    }
    pub(super) fn sync_property_dates(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        use gpui_component::date_picker::DateTime;
        let value = self.ui.property_value.read(cx).value();
        let date = chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d")
            .ok()
            .and_then(|date| date.and_hms_opt(0, 0, 0));
        let time = chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S")
            .ok()
            .or_else(|| chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M").ok());
        for (picker, value) in self
            .ui
            .property_dates
            .iter()
            .zip([date.or(time), time.or(date)])
        {
            picker.update(cx, |s, cx| s.set_date_time(DateTime::Single(value), w, cx));
        }
    }
    pub(super) fn accept_property_date(
        &mut self,
        index: usize,
        value: gpui_component::date_picker::DateTime,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use inkstone_core::properties::Kind;
        let kind = self.effective_property_kind(cx);
        if !self.ui.property_open || !matches!((index, kind), (0, Kind::Date) | (1, Kind::DateTime))
        {
            return;
        }
        let text = value
            .start()
            .map(|date| {
                date.format(if index == 0 {
                    "%Y-%m-%d"
                } else {
                    "%Y-%m-%dT%H:%M:%S"
                })
                .to_string()
            })
            .unwrap_or_default();
        self.ui
            .property_value
            .update(cx, |s, cx| s.set_value(text, w, cx));
        self.ui.property_error.clear();
        cx.notify();
    }
    pub(super) fn save_property(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.effective_property_kind(cx) == inkstone_core::properties::Kind::List
            && !self.add_property_list_item(w, cx)
        {
            return;
        }
        self.apply_property(false, w, cx);
    }
    pub(super) fn property_list_values(&self, cx: &Context<Self>) -> Result<Vec<String>, String> {
        let encoded = inkstone_core::properties::Kind::List
            .encode(&self.ui.property_value.read(cx).value())?;
        serde_json::from_str(&encoded).map_err(|e| e.to_string())
    }
    pub(super) fn add_property_list_item(
        &mut self,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.ui.property_open
            || self.effective_property_kind(cx) != inkstone_core::properties::Kind::List
        {
            return false;
        }
        let value = self
            .ui
            .property_list_entry
            .read(cx)
            .value()
            .trim()
            .to_string();
        if value.is_empty() {
            return true;
        }
        match self.property_list_values(cx) {
            Ok(mut items) => {
                items.push(value);
                self.ui.property_value.update(cx, |s, cx| {
                    s.set_value(serde_json::to_string(&items).unwrap(), w, cx)
                });
                self.ui.property_list_entry.update(cx, |s, cx| {
                    s.set_value("", w, cx);
                    s.focus(w, cx);
                });
                self.ui.property_error.clear();
                cx.notify();
                true
            }
            Err(error) => {
                self.ui.property_error = error;
                cx.notify();
                false
            }
        }
    }
    pub(super) fn remove_property_list_item(
        &mut self,
        index: usize,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(mut items) = self.property_list_values(cx)
            && self.ui.property_open
            && self.effective_property_kind(cx) == inkstone_core::properties::Kind::List
            && index < items.len()
        {
            items.remove(index);
            self.ui.property_value.update(cx, |s, cx| {
                s.set_value(serde_json::to_string(&items).unwrap(), w, cx)
            });
            self.ui.property_error.clear();
            cx.notify();
        }
    }
    pub(super) fn delete_property(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.apply_property(true, w, cx);
    }
    fn apply_property(&mut self, delete: bool, w: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.current_pane() else {
            return;
        };
        let text = pane.read(cx).editor.read(cx).value();
        if !self
            .ui
            .property_baseline
            .as_ref()
            .is_some_and(|(id, baseline)| {
                self.active
                    .and_then(|i| self.tabs.get(i))
                    .is_some_and(|tab| tab.id == *id)
                    && baseline == text.as_ref()
            })
        {
            self.status = "笔记已变更，请重新打开属性编辑。".into();
            self.ui.property_error = self.status.clone();
            cx.notify();
            return;
        }
        let result = if delete {
            self.ui
                .property_original
                .as_deref()
                .ok_or_else(|| "没有可删除的属性。".to_string())
                .and_then(|name| inkstone_core::properties::remove(&text, name))
        } else {
            let key = self.ui.property_key.read(cx).value();
            let kind = if matches!(key.as_ref(), "tags" | "aliases" | "cssclasses") {
                inkstone_core::properties::Kind::List
            } else {
                self.ui.property_kind
            };
            kind.encode(&self.ui.property_value.read(cx).value())
                .and_then(|value| {
                    inkstone_core::properties::edit(
                        &text,
                        self.ui.property_original.as_deref(),
                        &key,
                        &value,
                    )
                })
        };
        match result {
            Ok(text) => {
                if !delete {
                    let key = self.ui.property_key.read(cx).value().to_lowercase();
                    let kind = if matches!(key.as_str(), "tags" | "aliases" | "cssclasses") {
                        inkstone_core::properties::Kind::List
                    } else {
                        self.ui.property_kind
                    };
                    self.ui.prefs.property_types.insert(key, kind);
                    self.persist_workspace(cx);
                }
                pane.read(cx).editor.clone().update(cx, |s, cx| {
                    s.replace_all(text, w, cx);
                    s.focus(w, cx);
                });
                self.ui.property_open = false;
                self.ui.property_error.clear();
                self.ui.property_baseline = None;
                self.ui.property_original = None;
                self.status = if delete {
                    "属性已删除。"
                } else {
                    "属性已保存。"
                }
                .into();
            }
            Err(error) => {
                self.ui.property_error = error.clone();
                self.status = error;
            }
        }
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
                            markdown.push_str(&inkstone_core::locations::attachment_link(
                                &tab.path,
                                &path,
                                &this.index.files,
                                if catalog_error.is_some() {
                                    inkstone_core::locations::LinkFormat::Absolute
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
                let imported = !markdown.is_empty();
                if imported {
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
                if !imported || !errors.is_empty() {
                    // A failed import can still have copied some files.
                    this.schedule_auto_sync(true);
                }
                this.structure_changed = true;
                this.refresh_requested = true;
                this.sync_reference_contexts(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
