//! Vault selection, loading, and startup restoration.

use super::*;

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("新笔记名称.md"));
        let tree = cx.new(|cx| TreeState::new(cx));
        let tree_subscription = cx.subscribe(
            &tree,
            |this, _, event: &gpui_component::tree::TreeEvent, _| {
                use gpui_component::tree::TreeEvent;
                match event {
                    TreeEvent::Expanded(id) => {
                        let path = PathBuf::from(id.as_ref());
                        if !this.ui.prefs.expanded_folders.contains(&path) {
                            this.ui.prefs.expanded_folders.push(path);
                        }
                    }
                    TreeEvent::Collapsed(id) => this
                        .ui
                        .prefs
                        .expanded_folders
                        .retain(|p| p != std::path::Path::new(id.as_ref())),
                }
            },
        );
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名 / 全文"));
        let name_subscription = cx.subscribe_in(&name, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_name(window, cx);
            }
        });
        let search_subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.ui.selected = 0;
                this.ui.search_collapsed.clear();
                this.ui.modal_scroll.set_offset(Point::default());
                this.run_search(cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.open_selected_result(window, cx);
            }
        });
        let timer = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.request_window_close(window, cx))
                .unwrap_or(true)
        });
        let task = cx.background_executor().spawn(async move {
            if cfg!(test) {
                return None;
            }
            std::env::args_os().nth(1).map(PathBuf::from).or_else(|| {
                std::fs::read_to_string(app_dir().join("recent.txt"))
                    .ok()
                    .map(PathBuf::from)
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let root = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_startup(root, window, cx);
            });
        })
        .detach();
        let ui = ui::UiState::new(window, cx);
        Self {
            ui,
            #[cfg(target_os = "macos")]
            quit_requested: false,
            views: Default::default(),
            vault: None,
            files: vec![],
            tabs: vec![],
            active: None,
            next_id: 0,
            generation: 0,
            navigation_generation: 0,
            name,
            status: String::new(),
            loading: false,
            startup_pending: true,
            _timer: timer,
            watcher: None,
            watch_events: None,
            refresh_requested: false,
            refreshing: false,
            recoveries: vec![],
            index: Arc::new(Index::default()),
            link_paths: Default::default(),
            link_paths_key: None,
            search,
            search_results: vec![],
            search_revision: 0,
            fulltext: false,
            _search_subscription: search_subscription,
            _name_subscription: name_subscription,
            _tree_subscription: tree_subscription,
            pending_jump: None,
            pending_navigation: None,
            backlinks: vec![],
            backlink_scroll: UniformListScrollHandle::new(),
            tree,
            tree_files: vec![],
            command_open: false,
            changed_paths: Default::default(),
            known_writes: Default::default(),
            rescan: false,
            structure_changed: false,
            folder_revision: 0,
        }
    }
    pub(super) fn finish_startup(
        &mut self,
        root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A manual open may have already superseded the recent-vault lookup.
        if !std::mem::take(&mut self.startup_pending) {
            return;
        }
        if let Some(root) = root {
            self.load_vault(root, window, cx);
        }
        cx.notify();
    }

    pub(super) fn choose_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.pending_file_writes > 0
            || self
                .tabs
                .iter()
                .any(|t| t.save.dirty.get() || t.save.saving.get())
        {
            self.status = "请先保存当前笔记，再切换笔记库。".into();
            cx.notify();
            return;
        }
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开 Markdown 笔记库".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = dialog.await
                && let Some(root) = paths.first()
            {
                let root = root.clone();
                let _ = this.update_in(cx, |this, window, cx| this.load_vault(root, window, cx));
            }
        })
        .detach();
    }
    pub(super) fn load_vault(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.pending_file_writes > 0
            || self.ui.backup.pending.is_some()
            || self
                .tabs
                .iter()
                .any(|t| t.save.dirty.get() || t.save.saving.get())
        {
            self.status = "当前仍有未保存内容，已取消切换笔记库。".into();
            cx.notify();
            return;
        }
        self.cancel_queued_sync();
        inkstone_core::startup_trace::mark("load_requested");
        self.startup_pending = false;
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            inkstone_core::startup_trace::mark("background_started");
            let vault = Vault::open(root, app_dir().join("recovery"))?;
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut watcher = notify::recommended_watcher(move |event| {
                let _ = sender.send(event);
            })
            .map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
            watcher
                .watch(&vault.root, RecursiveMode::Recursive)
                .map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
            inkstone_core::startup_trace::mark("watcher_ready");
            let cache_path = Index::cache_path(&vault, &app_dir().join("index-cache"));
            let (index, folders) = Index::load_cached(&vault, &cache_path)?;
            inkstone_core::startup_trace::mark("index_ready");
            let files = index.note_paths();
            let recoveries = vault.recoveries()?;
            let _ = std::fs::write(
                app_dir().join("recent.txt"),
                vault.root.to_string_lossy().as_bytes(),
            );
            inkstone_core::startup_trace::mark("recovery_ready");
            inkstone_core::startup_trace::mark("folders_ready");
            let (prefs, preference_warning) =
                inkstone_core::preferences::Preferences::load_with_warning(
                    &vault.root.join(".inkstone-workspace.json"),
                );
            let cloud_secret = crate::workspace::webdav_secret::load(&prefs.webdav, &vault.root)
                .map_err(|error| error.to_string());
            inkstone_core::startup_trace::mark("preferences_read");
            let restored: Vec<_> = prefs
                .open_paths
                .iter()
                .enumerate()
                .filter_map(|(saved_index, path)| {
                    if path.as_os_str().is_empty() {
                        return Some((saved_index, path.clone(), String::new()));
                    }
                    vault
                        .read(path)
                        .ok()
                        .flatten()
                        .map(|text| (saved_index, path.clone(), text))
                })
                .collect();
            inkstone_core::startup_trace::mark("restored_texts_read");
            Ok::<_, VaultError>((
                vault,
                files,
                watcher,
                receiver,
                recoveries,
                index,
                folders,
                (prefs, preference_warning, cloud_secret),
                restored,
            ))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                if this.ui.pending_file_writes > 0
                    || this
                        .tabs
                        .iter()
                        .any(|t| t.save.dirty.get() || t.save.saving.get())
                {
                    this.status = "读取期间产生了新编辑，已保留当前笔记库。".into();
                    cx.notify();
                    return;
                }
                inkstone_core::startup_trace::mark("ui_apply_started");
                match result {
                    Ok((
                        vault,
                        files,
                        watcher,
                        receiver,
                        recoveries,
                        index,
                        folders,
                        prefs,
                        restored,
                    )) => {
                        let (prefs, preference_warning, cloud_secret) = prefs;
                        this.ui.folders = folders;
                        let restore_active = prefs.active_path.clone();
                        let restore_active_index = prefs.active_tab_index;
                        this.ui.prefs = prefs;
                        this.apply_cloud_secret(Some(cloud_secret), window, cx);
                        this.ui.font_size_slider.update(cx, |slider, cx| {
                            slider.set_value(this.ui.prefs.font_size, window, cx);
                        });
                        this.ui.tab_width.update(cx, |slider, cx| {
                            slider.set_value(this.ui.prefs.tab_size as f32, window, cx);
                        });
                        this.ui.left_mode = this.ui.prefs.left_panel;
                        this.ui.right_mode = this.ui.prefs.right_panel;
                        this.ui.tags_selected = None;
                        this.ui.search_collapsed.clear();
                        this.ui.search_signature = None;
                        this.fulltext = this.ui.left_mode == 1;
                        this.search.update(cx, |s, cx| {
                            s.set_value(this.ui.prefs.search_query.clone(), window, cx)
                        });
                        this.ui.tags_filter.update(cx, |s, cx| {
                            s.set_value(this.ui.prefs.tags.query.clone(), window, cx)
                        });
                        this.ui.link_update = None;
                        this.ui.history = None;
                        this.ui.conflict_review = None;
                        this.ui.backup = Default::default();
                        this.ui.exporting = false;
                        this.ui.attachment_manager = None;
                        this.ui.link_health = None;
                        this.ui.bulk_edit = None;
                        this.ui.table_editor = None;
                        this.ui.trash_open = false;
                        this.ui.last_persisted.clear();
                        this.ui.persist_error = None;
                        this.ui.discard_workspace_on_close = false;
                        this.ui.prefs.light =
                            this.ui.prefs.theme.is_light(Self::system_light(window));
                        ui::apply_theme(this.ui.prefs.light, cx);
                        this.apply_font_preferences(cx);
                        this.sync_font_selects(window, cx);
                        this.pending_navigation = None;
                        this.ui.closed.clear();
                        this.ui.inline_title = None;
                        this.ui.close_pending.clear();
                        this.ui.window_close_requested = false;
                        this.vault = Some(vault);
                        this.watcher = Some(watcher);
                        this.watch_events = Some(receiver);
                        this.recoveries = recoveries;
                        this.index = Arc::new(index);
                        let cached_index = this.index.clone();
                        let maintenance_vault = this.vault.as_ref().unwrap().clone();
                        cx.background_executor()
                            .spawn(async move {
                                let path = Index::cache_path(
                                    &maintenance_vault,
                                    &app_dir().join("index-cache"),
                                );
                                inkstone_core::startup_trace::mark("maintenance_started");
                                let _ = cached_index.save_cache(&maintenance_vault, &path);
                                inkstone_core::startup_trace::mark("cache_written");
                                let _ = maintenance_vault.cleanup_history();
                                inkstone_core::startup_trace::mark("history_cleaned");
                            })
                            .detach();
                        inkstone_core::startup_trace::mark("ui_preferences_ready");
                        this.sync_index_ui(cx);
                        inkstone_core::startup_trace::mark("tree_ready");
                        this.run_search(cx);
                        this.refreshing = false;
                        this.refresh_requested = false;
                        this.changed_paths.clear();
                        this.known_writes.clear();
                        this.rescan = false;
                        this.structure_changed = false;
                        this.files = files;
                        this.views = Default::default();
                        this.tabs.clear();
                        this.active = None;
                        this.status = preference_warning.unwrap_or_default();
                        if !this.recoveries.is_empty() {
                            this.status.push_str(&format!(
                                " 检测到 {} 条未保存草稿，可在命令面板的“查看回收站”中比较、恢复副本或放弃。",
                                this.recoveries.len()
                            ));
                        }
                        this.loading = true;
                        let mut restored_active = None;
                        let saved_views = this.ui.prefs.views.clone();
                        let mut used_views = std::collections::HashSet::new();
                        let mut view_restores = Vec::new();
                        let mut restored_slots = vec![None; this.ui.prefs.open_paths.len()];
                        inkstone_core::startup_trace::mark("tabs_restore_started");
                        for (saved_index, path, text) in restored {
                            let view_index = saved_views
                                .get(saved_index)
                                .filter(|view| {
                                    view.path == path && !used_views.contains(&saved_index)
                                })
                                .map(|_| saved_index)
                                .or_else(|| {
                                    saved_views
                                        .iter()
                                        .enumerate()
                                        .find(|(index, view)| {
                                            view.path == path && !used_views.contains(index)
                                        })
                                        .map(|(index, _)| index)
                                });
                            this.add_tab(path, Some(text), false, window, cx);
                            restored_slots[saved_index] = this.active;
                            if let Some(view_index) = view_index {
                                used_views.insert(view_index);
                                if let Some(tab) =
                                    this.active.and_then(|index| this.tabs.get(index))
                                {
                                    view_restores.push((tab.id, tab.pane.clone(), view_index));
                                }
                            }
                            if Some(saved_index) == restore_active_index {
                                restored_active = this.active;
                            }
                        }
                        inkstone_core::startup_trace::mark("tabs_created");
                        for (id, pane, view_index) in view_restores {
                            if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == id) {
                                tab.pinned = saved_views[view_index].pinned.unwrap_or(tab.pinned);
                            }
                            Self::restore_view_state(&pane, &saved_views[view_index], cx);
                        }
                        this.ui.prefs.main_tab_index = this
                            .ui
                            .prefs
                            .main_tab_index
                            .and_then(|index| restored_slots.get(index).copied().flatten());
                        this.ui.prefs.split_source_tab_index = this
                            .ui
                            .prefs
                            .split_source_tab_index
                            .and_then(|index| restored_slots.get(index).copied().flatten());
                        this.loading = false;
                        if let Some(i) = restored_active.or_else(|| {
                            restore_active.and_then(|p| this.tabs.iter().position(|t| t.path == p))
                        }) {
                            this.activate_tab(i, window, cx);
                        } else if this.tabs.is_empty()
                            && let Some(path) = this.files.first().cloned()
                        {
                            this.open_note(path, window, cx);
                        }
                        this.restore_split(window, cx);
                        inkstone_core::startup_trace::mark("ui_loaded");
                        if inkstone_core::startup_trace::enabled() {
                            cx.on_next_frame(window, |_, window, cx| {
                                cx.on_next_frame(window, |_, _, _| {
                                    inkstone_core::startup_trace::mark("loaded_frame_boundary");
                                });
                                cx.notify();
                            });
                        }
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
