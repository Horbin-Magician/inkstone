//! Opening notes and restoring their view state.

use super::*;

impl Workspace {
    pub(super) fn open_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_with_view(path, None, window, cx);
    }
    pub(super) fn apply_reopened_view(
        &mut self,
        state: Option<&inkstone_core::preferences::ViewState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = state
            && let Some(pane) = self.current_pane()
        {
            Self::restore_view_state(&pane, state, cx);
            pane.update(cx, |p, cx| p.focus_view(window, cx));
            self.persist_workspace(cx);
        }
    }
    pub(super) fn apply_reopened_tab(
        &mut self,
        closed: Option<&ui::ClosedTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(closed) = closed {
            self.apply_reopened_view(Some(&closed.view), window, cx);
            if let Some(pane) = self.current_pane() {
                pane.update(cx, |pane, cx| {
                    pane.navigation = closed.history.clone();
                    cx.notify();
                });
            }
        }
    }

    pub(super) fn open_existing_note(
        &mut self,
        index: usize,
        view: Option<&ui::ClosedTab>,
        force_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = view {
            if self.has_pending_input(self.tabs[index].id, window, cx) {
                self.ui.remember_closed(view.clone());
                self.status = "请完成当前编辑后再重新打开标签。".into();
                cx.notify();
                return;
            }
            self.views.secondary_focused = false;
            self.add_tab(view.view.path.clone(), None, false, window, cx);
            self.apply_reopened_tab(Some(view), window, cx);
        } else if force_new {
            if self.has_pending_input(self.tabs[index].id, window, cx) {
                self.pending_jump = None;
                self.status = "请完成当前编辑后再打开新标签。".into();
                cx.notify();
                return;
            }
            let path = self.tabs[index].path.clone();
            self.views.secondary_focused = false;
            self.add_tab(path, None, false, window, cx);
        } else {
            self.activate_tab(index, window, cx);
        }
    }

    pub(super) fn open_note_with_view(
        &mut self,
        path: PathBuf,
        view: Option<ui::ClosedTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_note_target(path, view, false, window, cx);
    }

    pub(super) fn open_link_note(
        &mut self,
        path: PathBuf,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_note_target(
            path,
            None,
            new_tab || self.current_view_pinned(),
            window,
            cx,
        );
    }

    pub(super) fn existing_note_target(&self, path: &std::path::Path) -> Option<usize> {
        // A document can have multiple independent views. Keep same-document
        // navigation in the active view so its caret and scroll state are used.
        self.active
            .filter(|&i| self.tabs.get(i).is_some_and(|tab| tab.path == path))
            .or_else(|| self.tabs.iter().position(|tab| tab.path == path))
    }

    pub(super) fn open_note_target(
        &mut self,
        path: PathBuf,
        view: Option<ui::ClosedTab>,
        force_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .ui
            .pending_command
            .as_ref()
            .is_some_and(|(p, _)| p != &path)
        {
            self.ui.pending_command = None;
        }
        if self
            .pending_jump
            .as_ref()
            .is_some_and(|(target, _)| target != &path)
        {
            self.pending_jump = None;
        }
        self.navigation_generation += 1;
        let navigation_generation = self.navigation_generation;
        self.pending_navigation = None;
        let force_new = force_new || (view.is_none() && self.current_view_pinned());
        if view.is_none() && !force_new && self.current_pane().is_some() {
            let history = self.navigation_with_current_state(cx);
            self.open_current_note(path, history, window, cx);
            return;
        }
        if let Some(i) = self.existing_note_target(&path) {
            self.open_existing_note(i, view.as_ref(), force_new, window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            if let Some(view) = view {
                self.ui.remember_closed(view);
            }
            return;
        };
        let generation = self.generation;
        let requested = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { vault.read(&requested) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation
                    || this.navigation_generation != navigation_generation
                {
                    if this.generation == generation
                        && let Some(view) = view
                    {
                        this.ui.remember_closed(view);
                    }
                    return;
                }
                if let Some(i) = this.existing_note_target(&path) {
                    this.open_existing_note(i, view.as_ref(), force_new, window, cx);
                    return;
                }
                match result {
                    Ok(Some(text)) => {
                        if force_new || view.is_some() {
                            this.views.secondary_focused = false;
                        }
                        this.add_tab(path, Some(text), false, window, cx);
                        this.apply_reopened_tab(view.as_ref(), window, cx);
                    }
                    Ok(None) => {
                        if let Some(view) = view {
                            this.ui.remember_closed(view);
                        }
                        this.status = "文件已不存在，请刷新目录。".into();
                        cx.notify();
                    }
                    Err(error) => {
                        if let Some(view) = view {
                            this.ui.remember_closed(view);
                        }
                        this.status = error.to_string();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn ensure_active_note(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .active
            .and_then(|i| self.tabs.get(i))
            .is_none_or(|t| t.path.as_os_str().is_empty())
        {
            self.focus_new(window, cx);
        }
        self.active
            .and_then(|i| self.tabs.get(i))
            .is_some_and(|t| !t.path.as_os_str().is_empty())
    }
    pub(super) fn new_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(PathBuf::new(), Some(String::new()), false, window, cx);
        window.focus(&self.ui.workspace_focus, cx);
    }
}
