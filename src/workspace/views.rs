use super::*;

pub(super) struct SplitPane {
    pub source: usize,
    pub pane: Entity<EditorPane>,
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

fn change(before: &str, after: &str) -> (std::ops::Range<usize>, String) {
    let start = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = before[start..]
        .chars()
        .rev()
        .zip(after[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    (
        start..before.len() - suffix,
        after[start..after.len() - suffix].to_string(),
    )
}
fn map_offset(offset: usize, range: &std::ops::Range<usize>, new_len: usize) -> usize {
    if offset <= range.start {
        offset
    } else if offset >= range.end {
        offset - range.len() + new_len
    } else {
        range.start + new_len
    }
}

impl Workspace {
    pub(super) fn request_window_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ui.inline_title.is_some() {
            self.commit_inline_title(false, window, cx);
            if self.ui.inline_title.is_some() && !self.ui.file_operation {
                self.ui.window_close_requested = false;
                self.status = "请完成或取消标题修改后关闭窗口。".into();
                cx.notify();
                return false;
            }
        }
        self.sync_from_split(window, cx);
        for tab in &mut self.tabs {
            tab.dirty =
                tab.baseline.as_deref() != Some(tab.pane.read(cx).editor.read(cx).value().as_ref());
        }
        if self.tabs.iter().any(|t| t.conflict || t.error.is_some()) {
            self.status = "请先处理保存冲突或另存副本，再关闭窗口。".into();
            cx.notify();
            return false;
        }
        self.snapshot_views(cx);
        let pending = self.ui.pending_file_writes > 0
            || self
                .tabs
                .iter()
                .any(|t| t.dirty || t.saving || self.has_pending_input(t.id, window, cx));
        let preferences_pending = self.vault.is_some()
            && (self.ui.persisting
                || serde_json::to_string(&self.ui.prefs)
                    .ok()
                    .is_none_or(|s| s != self.ui.last_persisted));
        if !pending && !preferences_pending {
            return true;
        }
        self.ui.window_close_requested = true;
        self.save_all(window, cx);
        self.persist_workspace(cx);
        self.status = "正在保存笔记与工作区，完成后关闭窗口…".into();
        cx.notify();
        false
    }
    pub(super) fn finish_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.window_close_requested {
            return;
        }
        if self.tabs.iter().any(|t| t.conflict || t.error.is_some()) {
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
    ) -> inkstone::preferences::ViewState {
        let pane = pane.read(cx);
        let editor = pane.editor.read(cx);
        let scroll = editor.scroll_offset();
        inkstone::preferences::ViewState {
            path,
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
        self.ui.prefs.views = self
            .tabs
            .iter()
            .map(|t| Self::snapshot_view(t.path.clone(), &t.pane, cx))
            .collect();
        self.ui.prefs.main_path = self
            .main_tab()
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        self.ui.prefs.split_view = self.views.split.as_ref().and_then(|s| {
            self.tabs
                .iter()
                .find(|t| t.id == s.source)
                .map(|t| Self::snapshot_view(t.path.clone(), &s.pane, cx))
        });
        self.ui.prefs.split_vertical = self.views.vertical;
        self.ui.prefs.split_focused = self.views.secondary_focused;
    }
    pub(super) fn restore_view_state(
        pane: &Entity<EditorPane>,
        state: &inkstone::preferences::ViewState,
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
                let parsed = inkstone::index::parse(&e.value());
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
        for state in &prefs.views {
            if let Some(tab) = self.tabs.iter().find(|t| t.path == state.path) {
                Self::restore_view_state(&tab.pane, state, cx);
            }
        }
        if let Some(state) = prefs.split_view
            && let Some(index) = self.tabs.iter().position(|t| t.path == state.path)
        {
            let main = prefs
                .main_path
                .and_then(|p| self.tabs.iter().position(|t| t.path == p))
                .unwrap_or(self.active.unwrap_or(index));
            self.views.main = Some(self.tabs[main].id);
            self.views.vertical = prefs.split_vertical;
            self.bind_split(index, window, cx);
            if let Some(split) = &self.views.split {
                Self::restore_view_state(&split.pane, &state, cx);
            }
            if !prefs.split_focused {
                self.focus_primary(main, window, cx);
            }
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        }
    }
    pub(super) fn has_pending_input(
        &self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
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
            .filter(|s| s.source == id)
            .is_some_and(|s| {
                if s.pane.update(cx, |p, cx| p.footnote_pending(window, cx)) {
                    return true;
                }
                let editor = s.pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            })
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
        self.views.secondary_focused = false;
        self.views.main = self.tabs.get(index).map(|t| t.id);
        self.activate_tab(index, window, cx);
    }
    pub(super) fn focus_secondary(&mut self, cx: &mut Context<Self>) {
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
        let canonical = tab.pane.read(cx).editor.clone();
        let text = canonical.read(cx).value();
        let selection = canonical.read(cx).selected_range();
        let original = tab.pane.read(cx);
        let (reading, live, image_dir) =
            (original.reading, original.live, original.image_dir.clone());
        let prefs = self.ui.prefs.clone();
        let paths = self.link_paths_for(&tab.path);
        let text_font = self.resolved_font(&prefs.text_font, "Microsoft YaHei UI");
        let pane = cx.new(|cx| {
            let mut p = EditorPane::new(&text, window, cx);
            p.reading = reading;
            p.live = live;
            p.image_dir = image_dir;
            p.font_size = prefs.font_size;
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
                tab.path.clone(),
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
            if matches!(event, InputEvent::Change)
                && this.views.split.as_ref().is_some_and(|s| {
                    s.source == id && s.pane.read(cx).editor.entity_id() == editor.entity_id()
                })
            {
                this.sync_from_split(w, cx);
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
            if !this.views.secondary_focused
                && editor.read(cx).focus_handle(cx).is_focused(w)
                && this.views.split.as_ref().is_some_and(|s| {
                    s.source == id && s.pane.read(cx).editor.entity_id() == editor.entity_id()
                })
            {
                this.focus_secondary(cx);
            }
        });
        let links = cx.subscribe_in(&pane, window, move |this, _, event, w, cx| {
            let Some(tab) = this.tabs.iter().find(|t| t.id == id) else {
                return;
            };
            let from = tab.path.clone();
            if !matches!(event, EditorEvent::CountsChanged) {
                this.focus_secondary(cx);
            }
            match event {
                EditorEvent::CountsChanged => cx.notify(),
                EditorEvent::FollowLink(target) => this.follow_link(from, target.clone(), w, cx),
                EditorEvent::FollowMarkdownLink(target) => {
                    this.follow_markdown_link(from, target, w, cx)
                }
                EditorEvent::FollowReference(reference) => {
                    this.follow_reference(reference.clone(), w, cx)
                }
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
            pane: pane.clone(),
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
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == source) else {
            return;
        };
        let canonical = tab.pane.read(cx).editor.clone();
        let before = canonical.read(cx).value();
        let after = editor.read(cx).value();
        if before == after {
            return;
        }
        let (range, replacement) = change(&before, &after);
        let len = replacement.len();
        canonical.update(cx, |state, cx| {
            let selection = state.selected_range();
            let scroll = state.scroll_offset();
            state.set_selected_range(range.clone(), cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(
                map_offset(selection.start, &range, len)..map_offset(selection.end, &range, len),
                cx,
            );
            state.set_scroll_offset(scroll, cx);
        });
        tab.dirty = tab.baseline.as_deref() != Some(after.as_ref());
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
        let after = tab.pane.read(cx).editor.read(cx).value();
        let editor = split.pane.read(cx).editor.clone();
        let before = editor.read(cx).value();
        if before == after {
            return;
        }
        // Composition belongs to the view receiving input; don't replace its preedit text.
        if editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
            return;
        }
        let (range, replacement) = change(&before, &after);
        let len = replacement.len();
        editor.update(cx, |state, cx| {
            let selection = state.selected_range();
            let scroll = state.scroll_offset();
            state.set_value(after, window, cx);
            state.set_selected_range(
                map_offset(selection.start, &range, len)..map_offset(selection.end, &range, len),
                cx,
            );
            state.set_scroll_offset(scroll, cx);
        });
    }
    pub(super) fn close_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|s| self.has_pending_input(s.source, window, cx))
        {
            self.status = "请完成输入法组词后关闭分屏。".into();
            cx.notify();
            return;
        }
        self.sync_from_split(window, cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[test]
    fn minimal_changes_preserve_unicode_boundaries() {
        assert_eq!(
            change("中文😀 hello", "中文😀 world"),
            ("中文😀 ".len().."中文😀 hello".len(), "world".into())
        );
        assert_eq!(change("a😀b", "a👩‍💻b"), (1..5, "👩‍💻".into()));
    }
}
