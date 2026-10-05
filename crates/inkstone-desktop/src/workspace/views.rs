use super::*;

pub(super) struct SplitPane {
    pub source: usize,
    pub pinned: bool,
    pub pane: Entity<EditorPane>,
    last_synced_text: SharedString,
    _changes: Subscription,
    _links: Subscription,
    _focus: Subscription,
}
#[derive(Default)]
pub(super) struct Views {
    pub split: Option<SplitPane>,
    pub main: Option<usize>,
    pub secondary_focused: bool,
    pub vertical: bool,
}

impl Workspace {
    pub(super) fn current_view_pinned(&self) -> bool {
        if self.views.secondary_focused {
            self.views.split.as_ref().is_some_and(|split| split.pinned)
        } else {
            self.active
                .and_then(|index| self.tabs.get(index))
                .is_some_and(|tab| tab.pinned)
        }
    }

    pub(super) fn open_current_note_in_new_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active.and_then(|index| self.tabs.get(index)) else {
            return;
        };
        let id = tab.id;
        let path = tab.path.clone();
        if path.as_os_str().is_empty() {
            return;
        }
        if self.has_pending_input(id, window, cx) {
            self.notifications
                .publish("请完成当前编辑后再打开新标签。".into());
            cx.notify();
            return;
        }
        self.document_view_changed(id, window, cx);
        self.sync_from_split(window, cx);
        let Some(pane) = self.current_pane() else {
            return;
        };
        let state = Self::snapshot_view(path.clone(), &pane, cx);
        let history = self.navigation_with_current_state(cx);
        self.views.secondary_focused = false;
        self.add_tab(path, None, false, window, cx);
        self.apply_reopened_view(Some(&state), window, cx);
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |pane, _| pane.navigation = history);
        }
        cx.notify();
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn request_app_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_window_close(window, cx) {
            window.remove_window();
        } else {
            self.quit_requested = self.ui.window_close_requested;
        }
    }

    pub(super) fn request_window_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ui.inline_title.is_some() {
            self.commit_inline_title(false, window, cx);
            if self.ui.inline_title.is_some() && !self.ui.file_operation {
                self.ui.window_close_requested = false;
                self.notifications
                    .publish("请完成或取消标题修改后关闭窗口。".into());
                cx.notify();
                return false;
            }
        }
        self.flush_document_views(window, cx);
        for tab in &mut self.tabs {
            tab.save
                .persistence
                .edited(tab.save.editor.read(cx).value().as_ref());
        }
        if self
            .tabs
            .iter()
            .any(|t| t.save.persistence.has_conflict() || t.save.persistence.error().is_some())
        {
            self.notifications
                .publish("请先处理保存冲突或另存副本，再关闭窗口。".into());
            cx.notify();
            return false;
        }
        self.snapshot_views(cx);
        self.prepare_cloud_sync_for_close(window, cx);
        let pending = self.ui.cloud_sync.is_pending()
            || self.ui.backup.pending.is_some()
            || self.ui.pending_file_writes > usize::from(self.ui.cloud_sync.is_busy())
            || self.tabs.iter().any(|t| {
                t.save.persistence.is_dirty()
                    || t.save.persistence.is_saving()
                    || self.has_pending_input(t.id, window, cx)
            });
        if self.settings_save.error().is_some() && !self.ui.discard_workspace_on_close {
            self.ui.window_close_requested = false;
            self.notifications
                .publish("工作区设置保存失败，请重试或选择“不保存布局并关闭”。".into());
            cx.notify();
            return false;
        }
        let preferences_pending = !self.ui.discard_workspace_on_close
            && self.vault.is_some()
            && (self.settings_save.is_busy()
                || serde_json::to_string(&self.ui.prefs)
                    .ok()
                    .is_none_or(|s| s != self.settings_save.last()));
        if !pending && !preferences_pending {
            return true;
        }
        self.ui.window_close_requested = true;
        self.save_all(window, cx);
        self.persist_workspace(cx);
        self.notifications
            .publish("正在保存笔记与工作区，完成后关闭窗口…".into());
        cx.notify();
        false
    }
    pub(super) fn finish_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.window_close_requested {
            #[cfg(target_os = "macos")]
            {
                self.quit_requested = false;
            }
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| t.save.persistence.has_conflict() || t.save.persistence.error().is_some())
        {
            self.ui.window_close_requested = false;
            return;
        }
        if self.request_window_close(window, cx) {
            self.ui.window_close_requested = false;
            window.remove_window();
        }
    }
    pub(super) fn snapshot_view(
        path: PathBuf,
        pane: &Entity<EditorPane>,
        cx: &App,
    ) -> inkstone_core::preferences::ViewState {
        let pane = pane.read(cx);
        let editor = pane.editor.read(cx);
        let scroll = editor.scroll_offset();
        inkstone_core::preferences::ViewState {
            path,
            pinned: None,
            reading: pane.reading,
            live: pane.live,
            selection: editor.selected_range(),
            scroll_x: f32::from(scroll.x),
            scroll_y: f32::from(scroll.y),
            folded_lines: editor
                .folded_ranges()
                .iter()
                .map(|f| f.start_line)
                .collect(),
            reading_position: Some(pane.reading_position(cx)),
            callout_states: pane.callout_states(cx),
        }
    }
    pub(super) fn snapshot_views(&mut self, cx: &App) {
        self.ui.prefs.main_tab_index = self.main_tab();
        self.ui.prefs.split_source_tab_index = self
            .views
            .split
            .as_ref()
            .and_then(|split| self.tabs.iter().position(|tab| tab.id == split.source));
        self.ui.prefs.views = self
            .tabs
            .iter()
            .map(|t| {
                let mut state = Self::snapshot_view(t.path.clone(), &t.pane, cx);
                state.pinned = Some(t.pinned);
                state
            })
            .collect();
        self.ui.prefs.main_path = self
            .main_tab()
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        self.ui.prefs.split_view = self.views.split.as_ref().and_then(|s| {
            self.tabs.iter().find(|t| t.id == s.source).map(|t| {
                let mut state = Self::snapshot_view(t.path.clone(), &s.pane, cx);
                state.pinned = Some(s.pinned);
                state
            })
        });
        self.ui.prefs.split_vertical = self.views.vertical;
        self.ui.prefs.split_focused = self.views.secondary_focused;
        self.ui.prefs.pinned_paths.clear();
    }
    pub(super) fn restore_view_state(
        pane: &Entity<EditorPane>,
        state: &inkstone_core::preferences::ViewState,
        cx: &mut App,
    ) {
        pane.update(cx, |p, cx| {
            p.reading = state.reading;
            p.live = state.live;
            p.restore_callout_states(&state.callout_states, cx);
            if let Some(position) = state.reading_position {
                p.restore_reading_position(position, cx);
            }
            p.editor.update(cx, |e, cx| {
                e.set_selected_range(state.selection.clone(), cx);
                let parsed = inkstone_core::index::parse(&e.value());
                e.apply_highlighter_fold_candidates(
                    parsed
                        .folds
                        .iter()
                        .map(|r| gpui_component::input::FoldRange::new(r.start, r.end))
                        .collect(),
                    cx,
                );
                e.restore_fold_lines(&state.folded_lines, cx);
                e.set_scroll_offset(point(px(state.scroll_x), px(state.scroll_y)), cx);
            });
            cx.notify();
        });
    }
    pub(super) fn restore_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = self.ui.prefs.clone();
        let was_loading = std::mem::replace(&mut self.loading, true);
        if let Some(state) = prefs.split_view
            && let Some(index) = prefs
                .split_source_tab_index
                .filter(|index| {
                    self.tabs
                        .get(*index)
                        .is_some_and(|tab| tab.path == state.path)
                })
                .or_else(|| self.tabs.iter().position(|t| t.path == state.path))
        {
            let main = prefs
                .main_tab_index
                .filter(|index| {
                    self.tabs.get(*index).is_some_and(|tab| {
                        prefs
                            .main_path
                            .as_ref()
                            .is_none_or(|path| tab.path == *path)
                    })
                })
                .or_else(|| {
                    prefs
                        .main_path
                        .and_then(|p| self.tabs.iter().position(|t| t.path == p))
                })
                .unwrap_or(self.active.unwrap_or(index));
            self.views.main = Some(self.tabs[main].id);
            self.views.vertical = prefs.split_vertical;
            self.bind_split(index, window, cx);
            if let Some(split) = &mut self.views.split {
                split.pinned = state.pinned.unwrap_or(false);
                Self::restore_view_state(&split.pane, &state, cx);
            }
            if !prefs.split_focused {
                self.focus_primary(main, window, cx);
            }
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        }
        self.loading = was_loading;
        self.persist_workspace(cx);
    }
    pub(super) fn has_pending_input(
        &self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.save.clone())
        else {
            return false;
        };
        if document.editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        }) {
            return true;
        }
        for tab in self
            .tabs
            .iter()
            .filter(|tab| std::rc::Rc::ptr_eq(&tab.save, &document))
        {
            if tab.pane.update(cx, |p, cx| p.footnote_pending(window, cx)) {
                return true;
            }
            let editor = tab.pane.read(cx).editor.clone();
            if editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
                return true;
            }
        }
        self.views
            .split
            .as_ref()
            .filter(|s| {
                self.tabs
                    .iter()
                    .any(|tab| tab.id == s.source && std::rc::Rc::ptr_eq(&tab.save, &document))
            })
            .is_some_and(|s| {
                if s.pane.update(cx, |p, cx| p.footnote_pending(window, cx)) {
                    return true;
                }
                let editor = s.pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            })
    }
    pub(super) fn remove_missing_views(&mut self) {
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|split| !self.tabs.iter().any(|tab| tab.id == split.source))
        {
            self.views.split = None;
            self.views.secondary_focused = false;
        }
        if self
            .views
            .main
            .is_some_and(|id| !self.tabs.iter().any(|tab| tab.id == id))
        {
            self.views.main = self
                .active
                .and_then(|index| self.tabs.get(index))
                .map(|tab| tab.id);
        }
    }

    pub(super) fn current_pane(&self) -> Option<Entity<EditorPane>> {
        if self.views.secondary_focused
            && let Some(split) = &self.views.split
        {
            return Some(split.pane.clone());
        }
        self.active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.pane.clone())
    }
    pub(super) fn main_tab(&self) -> Option<usize> {
        if self.views.split.is_some() {
            self.views
                .main
                .and_then(|id| self.tabs.iter().position(|t| t.id == id))
        } else {
            self.active
        }
    }
    pub(super) fn focus_primary(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.views.secondary_focused {
            self.navigation_generation += 1;
            self.pending_navigation = None;
        }
        self.views.secondary_focused = false;
        self.views.main = self.tabs.get(index).map(|t| t.id);
        self.activate_tab(index, window, cx);
    }
    pub(super) fn focus_secondary(&mut self, cx: &mut Context<Self>) {
        if !self.views.secondary_focused {
            self.navigation_generation += 1;
            self.pending_navigation = None;
        }
        if let Some(split) = &self.views.split {
            self.active = self.tabs.iter().position(|t| t.id == split.source);
            self.views.secondary_focused = true;
            self.run_search(cx);
            cx.notify();
        }
    }
    pub(super) fn split_active(
        &mut self,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active else {
            return;
        };
        if self.has_pending_input(self.tabs[index].id, window, cx) {
            self.notifications
                .publish("请完成当前编辑后再操作分屏。".into());
            cx.notify();
            return;
        }
        if self.views.split.is_none() {
            self.views.main = Some(self.tabs[index].id);
        }
        self.views.vertical = vertical;
        if let Some(split) = &self.views.split
            && split.source == self.tabs[index].id
        {
            self.views.secondary_focused = true;
            split
                .pane
                .clone()
                .update(cx, |p, cx| p.focus_view(window, cx));
            self.persist_workspace(cx);
            cx.notify();
            return;
        }
        self.bind_split(index, window, cx);
    }
    pub(super) fn bind_split(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let id = tab.id;
        let tab_path = tab.path.clone();
        let canonical = tab.save.editor.clone();
        let text = canonical.read(cx).value();
        let selection = tab.pane.read(cx).editor.read(cx).selected_range();
        let original = tab.pane.read(cx);
        let (reading, live, image_dir) =
            (original.reading, original.live, original.image_dir.clone());
        let prefs = self.ui.prefs.clone();
        let path = tab_path.clone();
        let original_navigation = original.navigation.clone();
        let paths = self.cached_link_paths(&path);
        let text_font = self.resolved_font(&prefs.text_font, "Microsoft YaHei UI");
        let mut navigation = self
            .views
            .split
            .as_ref()
            .map(|split| split.pane.read(cx).navigation.clone())
            .unwrap_or(original_navigation);
        navigation.visit(path);
        let pane = cx.new(|cx| {
            let mut p = EditorPane::new(&text, window, cx);
            p.navigation = navigation;
            p.reading = reading;
            p.live = live;
            p.image_dir = image_dir;
            p.font_size = prefs.font_size;
            p.quick_font_size = prefs.quick_font_size;
            p.text_font = text_font;
            p.light = prefs.light;
            p.readable_width = prefs.readable_width;
            p.strict_line_breaks = prefs.strict_line_breaks;
            p.smart_lists = prefs.smart_lists;
            p.set_fold_options(prefs.fold_headings, prefs.fold_indentation, window, cx);
            p.set_auto_pairing(prefs.auto_pair_brackets, prefs.auto_pair_markdown, cx);
            p.indentation = gpui_base::input::TabSize {
                tab_size: prefs.tab_size.clamp(2, 8),
                hard_tabs: prefs.use_tabs,
            };
            p.history_owner = Some(canonical);
            p.set_paths(paths);
            p.editor.update(cx, |s, cx| {
                s.set_line_number(prefs.line_numbers, window, cx);
                s.set_indent_guides(prefs.show_indent_guides, window, cx);
                s.set_tab_size(p.indentation, cx);
                s.set_selected_range(selection, cx);
            });
            p
        });
        pane.update(cx, |p, cx| {
            p.set_reference_context(
                tab_path,
                self.vault
                    .as_ref()
                    .map(|v| v.root.clone())
                    .unwrap_or_default(),
                self.index.clone(),
                cx,
            )
        });
        let editor = pane.read(cx).editor.clone();
        let changes = cx.subscribe_in(&editor, window, move |this, editor, event, w, cx| {
            if matches!(event, InputEvent::Change) {
                if this
                    .views
                    .split
                    .as_ref()
                    .is_some_and(|s| s.pane.read(cx).editor.entity_id() == editor.entity_id())
                {
                    this.sync_from_split(w, cx);
                } else if let Some(id) = this
                    .tabs
                    .iter()
                    .find(|tab| tab.pane.read(cx).editor.entity_id() == editor.entity_id())
                    .map(|tab| tab.id)
                {
                    this.document_view_changed(id, w, cx);
                }
            }
        });
        let mut counted_selection = 0..0;
        let focus = cx.observe_in(&editor, window, move |this, editor, w, cx| {
            let selected = editor.read(cx).selected_range();
            let selected = if selected.is_empty() { 0..0 } else { selected };
            if selected != counted_selection {
                counted_selection = selected;
                if editor.read(cx).focus_handle(cx).is_focused(w) {
                    cx.notify();
                }
            }
            if editor.read(cx).focus_handle(cx).is_focused(w) {
                if this
                    .views
                    .split
                    .as_ref()
                    .is_some_and(|s| s.pane.read(cx).editor.entity_id() == editor.entity_id())
                {
                    if !this.views.secondary_focused {
                        this.focus_secondary(cx);
                    }
                } else if let Some(index) = this
                    .tabs
                    .iter()
                    .position(|tab| tab.pane.read(cx).editor.entity_id() == editor.entity_id())
                    && (this.views.secondary_focused || this.active != Some(index))
                {
                    this.focus_primary(index, w, cx);
                }
            }
        });
        let links = cx.subscribe_in(&pane, window, move |this, pane, event, w, cx| {
            let split_source = this
                .views
                .split
                .as_ref()
                .filter(|split| split.pane.entity_id() == pane.entity_id())
                .map(|split| split.source);
            let Some(index) = this.tabs.iter().position(|tab| {
                Some(tab.id) == split_source || tab.pane.entity_id() == pane.entity_id()
            }) else {
                return;
            };
            let from = this.tabs[index].path.clone();
            if !matches!(
                event,
                EditorEvent::CountsChanged | EditorEvent::FontSizeDelta(_)
            ) {
                if split_source.is_some() {
                    this.focus_secondary(cx);
                } else {
                    this.focus_primary(index, w, cx);
                }
            }
            match event {
                EditorEvent::CountsChanged => cx.notify(),
                EditorEvent::FontSizeDelta(delta) => this.adjust_font_size(*delta, w, cx),
                EditorEvent::FollowLink(target) | EditorEvent::FollowLinkInNewTab(target) => this
                    .follow_link(
                        from,
                        target.clone(),
                        matches!(event, EditorEvent::FollowLinkInNewTab(_)),
                        w,
                        cx,
                    ),
                EditorEvent::FollowMarkdownLink(target)
                | EditorEvent::FollowMarkdownLinkInNewTab(target) => this.follow_markdown_link(
                    from,
                    target,
                    matches!(event, EditorEvent::FollowMarkdownLinkInNewTab(_)),
                    w,
                    cx,
                ),
                EditorEvent::FollowReference(reference)
                | EditorEvent::FollowReferenceInNewTab(reference) => this.follow_reference(
                    reference.clone(),
                    matches!(event, EditorEvent::FollowReferenceInNewTab(_)),
                    w,
                    cx,
                ),
                EditorEvent::ToggleTask(target, checked) => {
                    this.toggle_referenced_task(target.clone(), *checked, w, cx)
                }
                EditorEvent::PasteFiles(paths) => this.import_files(paths.clone(), w, cx),
                EditorEvent::PasteImage(name, bytes) => {
                    this.paste_image(name.clone(), bytes.clone(), w, cx)
                }
            }
        });
        self.views.split = Some(SplitPane {
            source: id,
            pinned: false,
            pane: pane.clone(),
            last_synced_text: text,
            _changes: changes,
            _links: links,
            _focus: focus,
        });
        self.active = Some(index);
        self.views.secondary_focused = true;
        pane.update(cx, |p, cx| p.focus_view(window, cx));
        self.persist_workspace(cx);
        cx.notify();
    }
    pub(super) fn sync_from_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(split) = &self.views.split else {
            return;
        };
        let source = split.source;
        let editor = split.pane.read(cx).editor.clone();
        if editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
            return;
        }
        let after = editor.read(cx).value();
        if after == split.last_synced_text {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == source) else {
            return;
        };
        let canonical = tab.save.editor.clone();
        let before = canonical.read(cx).value();
        if before == after {
            self.views.split.as_mut().unwrap().last_synced_text = after;
            return;
        }
        let edits = inkstone_core::text_changes::diff(&before, &after);
        let history = editor.read(cx).history_group_id().map_or(
            gpui_base::input::SyncedHistory::Record,
            gpui_base::input::SyncedHistory::Group,
        );
        if !canonical.update(cx, |state, cx| {
            state.apply_synced_text(&after, &edits, history, false, window, cx)
        }) {
            return;
        }
        tab.save.persistence.edited(after.as_ref());
        self.views.split.as_mut().unwrap().last_synced_text = after;
        cx.notify();
    }
    pub(super) fn sync_to_split(
        &mut self,
        source: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(split) = self.views.split.as_ref().filter(|s| s.source == source) else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|t| t.id == source) else {
            return;
        };
        let canonical = tab.save.editor.clone();
        if canonical.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let after = canonical.read(cx).value();
        if after == split.last_synced_text {
            return;
        }
        let editor = split.pane.read(cx).editor.clone();
        let before = editor.read(cx).value();
        // Composition belongs to the view receiving input; don't replace its preedit text.
        if editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
            return;
        }
        if before == after {
            self.views.split.as_mut().unwrap().last_synced_text = after;
            return;
        }
        let edits = inkstone_core::text_changes::diff(&before, &after);
        let applied = editor.update(cx, |state, cx| {
            state.apply_synced_text(
                &after,
                &edits,
                gpui_base::input::SyncedHistory::Ignore,
                self.views.secondary_focused,
                window,
                cx,
            )
        });
        if applied {
            self.views.split.as_mut().unwrap().last_synced_text = after;
        }
    }
    pub(super) fn promote_split_view(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = self.tabs[index].id;
        if self.views.main != Some(id)
            || !self
                .views
                .split
                .as_ref()
                .is_some_and(|split| split.source == id)
        {
            return false;
        }
        let split = self.views.split.take().unwrap();
        let tab = &mut self.tabs[index];
        let closed = Self::snapshot_view(tab.path.clone(), &tab.pane, cx);
        if !tab.path.as_os_str().is_empty() {
            self.ui.remember_closed(ui::ClosedTab::new(
                closed,
                tab.pane.read(cx).navigation.clone(),
            ));
        }
        tab.pane = split.pane;
        tab.pinned = split.pinned;
        tab.synced_text = split.last_synced_text;
        tab._subscription = Some(split._changes);
        tab._links = split._links;
        tab._focus = split._focus;
        self.ui.close_pending.remove(&id);
        self.views.secondary_focused = false;
        self.active = Some(index);
        tab.pane.update(cx, |pane, cx| pane.focus_view(window, cx));
        self.run_search(cx);
        self.persist_workspace(cx);
        cx.notify();
        true
    }

    pub(super) fn close_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(split) = &mut self.views.split
            && split.pinned
        {
            split.pinned = false;
            self.persist_workspace(cx);
            cx.notify();
            return;
        }
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|s| self.has_pending_input(s.source, window, cx))
        {
            self.notifications
                .publish("请完成输入法组词后关闭分屏。".into());
            cx.notify();
            return;
        }
        self.sync_from_split(window, cx);
        if let Some(split) = &self.views.split
            && let Some(tab) = self.tabs.iter().find(|tab| tab.id == split.source)
            && !tab.path.as_os_str().is_empty()
        {
            self.ui.remember_closed(ui::ClosedTab::new(
                Self::snapshot_view(tab.path.clone(), &split.pane, cx),
                split.pane.read(cx).navigation.clone(),
            ));
        }
        self.views.split = None;
        self.views.secondary_focused = false;
        if let Some(index) = self
            .views
            .main
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
        {
            self.activate_tab(index, window, cx);
        }
        self.persist_workspace(cx);
        cx.notify();
    }
}
