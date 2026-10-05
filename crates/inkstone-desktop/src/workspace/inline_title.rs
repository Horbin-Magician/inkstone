use super::*;

pub(super) struct InlineTitle {
    pub id: usize,
    pub input: Entity<InputState>,
    pub focus_after: bool,
    _subscription: Subscription,
}

impl Workspace {
    #[cfg(test)]
    pub(super) fn begin_inline_title(
        &mut self,
        index: usize,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.file_operation || self.ui.link_update.is_some() {
            return;
        }
        if let Some(edit) = &self.ui.inline_title {
            edit.input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let Some(tab) = self
            .tabs
            .get(index)
            .filter(|tab| !tab.path.as_os_str().is_empty())
        else {
            return;
        };
        let id = tab.id;
        let title = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if secondary {
            self.focus_secondary(cx);
        } else {
            self.focus_primary(index, window, cx);
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title.clone()));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_inline_title(
                    matches!(event, InputEvent::PressEnter { .. }),
                    window,
                    cx,
                );
            }
        });
        self.ui.inline_title = Some(InlineTitle {
            id,
            input: input.clone(),
            focus_after: false,
            _subscription: subscription,
        });
        input.update(cx, |s, cx| {
            s.set_selected_range(0..title.len(), cx);
            s.focus(window, cx);
        });
        cx.notify();
    }
    pub(super) fn cancel_inline_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.inline_title = None;
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        }
        cx.notify();
    }
    pub(super) fn commit_inline_title(
        &mut self,
        focus_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.file_operation {
            return;
        }
        let Some(edit) = &self.ui.inline_title else {
            return;
        };
        if edit
            .input
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        {
            return;
        }
        let id = edit.id;
        let title = edit.input.read(cx).value().trim().to_string();
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else {
            self.ui.inline_title = None;
            return;
        };
        if tab
            .path
            .file_stem()
            .is_some_and(|name| name.to_string_lossy() == title)
        {
            if focus_after {
                self.cancel_inline_title(window, cx);
            } else {
                self.ui.inline_title = None;
                cx.notify();
            }
            return;
        }
        if title.is_empty() || title.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
            self.notifications
                .publish("文件名不能为空或包含路径保留字符。".into());
            cx.notify();
            return;
        }
        let destination = tab
            .path
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .join(format!("{title}.md"));
        if let Some(edit) = &mut self.ui.inline_title {
            edit.focus_after = focus_after;
        }
        self.manage_named_note(
            id,
            false,
            destination.to_string_lossy().to_string(),
            window,
            cx,
        );
    }
}
