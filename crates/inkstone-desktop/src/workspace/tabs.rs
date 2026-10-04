//! Tab creation, activation, and deferred closing.

use super::*;

impl Workspace {
    pub(super) fn add_tab(
        &mut self,
        path: PathBuf,
        baseline: Option<String>,
        new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.inline_title.is_some() {
            self.commit_inline_title(false, window, cx);
        }
        if !self.loading
            && !path.as_os_str().is_empty()
            && !self.views.secondary_focused
            && let Some(i) = self.active.filter(|i| {
                self.tabs
                    .get(*i)
                    .is_some_and(|t| t.path.as_os_str().is_empty())
            })
        {
            self.tabs.remove(i);
            self.active = None;
            self.views.main = None;
        }
        let target_split = self.views.secondary_focused && self.views.split.is_some();
        let shared_document = self
            .tabs
            .iter()
            .find(|tab| !new && !path.as_os_str().is_empty() && tab.path == path)
            .map(|tab| tab.save.clone());
        if shared_document.as_ref().is_some_and(|document| {
            document.editor.update(cx, |editor, cx| {
                editor.marked_text_range(window, cx).is_some()
            })
        }) {
            self.status = "请完成输入法组词后再打开另一个视图。".into();
            cx.notify();
            return;
        }
        let initial_text = shared_document
            .as_ref()
            .map(|document| document.editor.read(cx).value())
            .unwrap_or_else(|| baseline.clone().unwrap_or_default().into());
        let pane = cx.new(|cx| {
            let mut pane = EditorPane::new(&initial_text, window, cx);
            if !path.as_os_str().is_empty() {
                pane.navigation.visit(path.clone());
            }
            pane.live = self.ui.prefs.default_live_preview;
            pane.reading = self.ui.prefs.default_reading && !new && !path.as_os_str().is_empty();
            pane
        });
        pane.update(cx, |pane, _| pane.set_paths(self.link_paths_for(&path)));
        if let Some(vault) = &self.vault {
            pane.update(cx, |pane, _| {
                pane.image_dir = vault
                    .root
                    .join(path.parent().unwrap_or(std::path::Path::new("")))
            });
        }
        let editor = pane.read(cx).editor.clone();
        let id = self.next_id;
        self.next_id += 1;
        let links = cx.subscribe_in(&pane, window, move |this, _, event, window, cx| {
            if let Some(index) = this.tabs.iter().position(|t| t.id == id) {
                let from = this.tabs[index].path.clone();
                if matches!(
                    event,
                    EditorEvent::FollowLink(_)
                        | EditorEvent::FollowMarkdownLink(_)
                        | EditorEvent::FollowReference(_)
                        | EditorEvent::FollowLinkInNewTab(_)
                        | EditorEvent::FollowMarkdownLinkInNewTab(_)
                        | EditorEvent::FollowReferenceInNewTab(_)
                ) {
                    this.focus_primary(index, window, cx);
                }
                match event {
                    EditorEvent::CountsChanged => cx.notify(),
                    EditorEvent::FontSizeDelta(delta) => this.adjust_font_size(*delta, window, cx),
                    EditorEvent::FollowLink(target) | EditorEvent::FollowLinkInNewTab(target) => {
                        this.follow_link(
                            from,
                            target.clone(),
                            matches!(event, EditorEvent::FollowLinkInNewTab(_)),
                            window,
                            cx,
                        )
                    }
                    EditorEvent::FollowMarkdownLink(target)
                    | EditorEvent::FollowMarkdownLinkInNewTab(target) => this.follow_markdown_link(
                        from,
                        target,
                        matches!(event, EditorEvent::FollowMarkdownLinkInNewTab(_)),
                        window,
                        cx,
                    ),
                    EditorEvent::FollowReference(reference)
                    | EditorEvent::FollowReferenceInNewTab(reference) => this.follow_reference(
                        reference.clone(),
                        matches!(event, EditorEvent::FollowReferenceInNewTab(_)),
                        window,
                        cx,
                    ),
                    EditorEvent::ToggleTask(target, checked) => {
                        this.toggle_referenced_task(target.clone(), *checked, window, cx)
                    }
                    EditorEvent::PasteFiles(paths) => this.import_files(paths.clone(), window, cx),
                    EditorEvent::PasteImage(name, bytes) => {
                        this.paste_image(name.clone(), bytes.clone(), window, cx)
                    }
                }
            }
        });
        let (save, subscription) = if let Some(document) = shared_document {
            pane.update(cx, |pane, _| {
                pane.history_owner = Some(document.editor.clone())
            });
            let subscription =
                cx.subscribe_in(&editor, window, move |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.document_view_changed(id, window, cx);
                    }
                });
            (document, Some(subscription))
        } else {
            let changes =
                cx.subscribe_in(&editor, window, move |this, editor, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.document_changed(editor.clone(), window, cx);
                    }
                });
            (
                std::rc::Rc::new(document::DocumentState::new(
                    path.clone(),
                    baseline,
                    new,
                    editor.clone(),
                    changes,
                )),
                None,
            )
        };
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
            if editor.read(cx).focus_handle(cx).is_focused(w)
                && this.views.split.is_some()
                && (this.views.secondary_focused || this.views.main != Some(id))
            {
                this.views.secondary_focused = false;
                this.views.main = Some(id);
                this.active = this.tabs.iter().position(|t| t.id == id);
                this.run_search(cx);
                cx.notify();
            }
        });
        self.tabs.push(Tab {
            id,
            pinned: self.loading && self.ui.prefs.pinned_paths.contains(&path),
            path,
            pane,
            save,
            synced_text: initial_text,
            _subscription: subscription,
            _links: links,
            _focus: focus,
        });
        self.sync_reference_contexts(cx);
        self.active = Some(self.tabs.len() - 1);
        if target_split {
            self.bind_split(self.tabs.len() - 1, window, cx);
        } else {
            self.views.main = Some(id);
        }
        self.close_quick_search(window, cx);
        self.ui.name_mode = None;
        self.apply_editor_preferences(window, cx);
        if let Some((path, command)) = self.ui.pending_command.take()
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path == path)
        {
            self.execute_command(command, window, cx);
        }
        self.apply_jump(window, cx);
        self.run_search(cx);
        cx.notify();
    }
    pub(super) fn activate_tab(
        &mut self,
        mut index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active != Some(index) {
            self.navigation_generation += 1;
            self.pending_navigation = None;
        }
        let id = self.tabs[index].id;
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| edit.id != id)
        {
            self.commit_inline_title(false, window, cx);
        }
        if !self.views.secondary_focused
            && !self.tabs[index].path.as_os_str().is_empty()
            && let Some(i) = self.active.filter(|i| {
                self.tabs
                    .get(*i)
                    .is_some_and(|t| t.path.as_os_str().is_empty())
            })
        {
            self.tabs.remove(i);
            index = self.tabs.iter().position(|t| t.id == id).unwrap();
        }
        self.active = Some(index);
        self.close_quick_search(window, cx);
        if self.views.secondary_focused && self.views.split.is_some() {
            if self
                .views
                .split
                .as_ref()
                .is_some_and(|s| s.source == self.tabs[index].id)
            {
                self.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .clone()
                    .update(cx, |p, cx| p.focus_view(window, cx));
            } else {
                self.bind_split(index, window, cx);
            }
        } else {
            self.views.main = Some(self.tabs[index].id);
            self.tabs[index]
                .pane
                .update(cx, |p, cx| p.focus_view(window, cx));
        }
        self.run_search(cx);
        if let Some((path, command)) = self.ui.pending_command.take()
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path == path)
        {
            self.execute_command(command, window, cx);
        }
        cx.notify();
    }
    pub(super) fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.views.secondary_focused && self.views.split.is_some() {
            self.close_split(window, cx);
            return;
        }
        if let Some(index) = self.active {
            self.close_tab_at(index, window, cx);
        }
    }
    pub(super) fn close_tab_group(
        &mut self,
        anchor: Option<usize>,
        mode: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor_index = anchor.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if mode != 2 && anchor_index.is_none() {
            return;
        }
        let ids: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                !t.pinned
                    && match mode {
                        0 => Some(t.id) != anchor,
                        1 => Some(*i) > anchor_index,
                        _ => true,
                    }
            })
            .map(|(_, t)| t.id)
            .collect();
        for id in ids.into_iter().rev() {
            if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                self.close_tab_at(i, window, cx);
            }
        }
        if mode == 0
            && let Some(index) = anchor.and_then(|id| self.tabs.iter().position(|t| t.id == id))
        {
            self.focus_primary(index, window, cx);
        }
        self.persist_workspace(cx);
        cx.notify();
    }
    pub(super) fn close_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| self.tabs.get(index).is_some_and(|tab| tab.id == edit.id))
        {
            self.commit_inline_title(false, window, cx);
            if self.ui.inline_title.is_some() && !self.ui.file_operation {
                self.status = "请完成或取消标题修改后关闭标签。".into();
                cx.notify();
                return;
            }
        }
        if let Some(id) = self.tabs.get(index).map(|t| t.id)
            && self.has_pending_input(id, window, cx)
        {
            self.status = "请完成输入法组词后关闭标签。".into();
            cx.notify();
            return;
        }
        if let Some(id) = self.tabs.get(index).map(|tab| tab.id) {
            self.document_view_changed(id, window, cx);
        }
        self.sync_from_split(window, cx);
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if tab.pinned {
            tab.pinned = false;
            self.persist_workspace(cx);
            cx.notify();
            return;
        }
        let document = tab.save.clone();
        if self
            .tabs
            .iter()
            .enumerate()
            .any(|(other, tab)| other != index && std::rc::Rc::ptr_eq(&tab.save, &document))
        {
            self.remove_tab_view(index, window, cx);
            return;
        }
        if self.promote_split_view(index, window, cx) {
            return;
        }
        let tab = &self.tabs[index];
        tab.save.dirty.set(
            tab.save.baseline.borrow().as_deref()
                != Some(tab.save.editor.read(cx).value().as_ref()),
        );
        if tab.save.conflict.get() || tab.save.error.borrow().is_some() {
            self.status = "请先处理保存错误或另存副本，再关闭标签。".into();
            cx.notify();
            return;
        }
        if tab.save.dirty.get() || tab.save.saving.get() {
            self.ui.close_pending.insert(tab.id);
            self.status = "正在保存，成功后关闭标签…".into();
            self.save_all(window, cx);
            cx.notify();
            return;
        }
        self.remove_tab_view(index, window, cx);
    }
    pub(super) fn remove_tab_view(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let main_before = self.views.main;
        let active_id = self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id);
        let removed = self.tabs.remove(index);
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| edit.id == removed.id)
        {
            self.ui.inline_title = None;
        }
        let closed_view = Self::snapshot_view(removed.path.clone(), &removed.pane, cx);
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|s| s.source == removed.id)
        {
            if let Some(peer) = self
                .tabs
                .iter()
                .find(|tab| std::rc::Rc::ptr_eq(&tab.save, &removed.save))
            {
                self.views.split.as_mut().unwrap().source = peer.id;
            } else {
                self.views.split = None;
                self.views.secondary_focused = false;
            }
        }
        self.ui.close_pending.remove(&removed.id);
        if !removed.path.as_os_str().is_empty() {
            self.ui.remember_closed(ui::ClosedTab::new(
                closed_view,
                removed.pane.read(cx).navigation.clone(),
            ));
        }
        self.active = active_id
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .or_else(|| {
                (!self.tabs.is_empty()).then_some(index.min(self.tabs.len().saturating_sub(1)))
            });
        if self.views.secondary_focused
            && let Some(split) = &self.views.split
        {
            self.active = self.tabs.iter().position(|tab| tab.id == split.source);
        }
        self.views.main = main_before
            .filter(|id| self.tabs.iter().any(|t| t.id == *id))
            .or_else(|| self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id));
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        } else {
            window.focus(&self.ui.workspace_focus, cx);
        }
        self.run_search(cx);
        self.persist_workspace(cx);
        cx.notify();
    }
    pub(super) fn finish_pending_closes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if tab.pinned {
                self.ui.close_pending.remove(&tab.id);
            }
        }
        let ready: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                self.ui.close_pending.contains(&t.id)
                    && !t.save.dirty.get()
                    && !t.save.saving.get()
                    && !t.save.conflict.get()
                    && t.save.error.borrow().is_none()
                    && !t.pinned
                    && !self.has_pending_input(t.id, window, cx)
            })
            .map(|(i, _)| i)
            .collect();
        for i in ready.into_iter().rev() {
            self.remove_tab_view(i, window, cx);
        }
    }
}
