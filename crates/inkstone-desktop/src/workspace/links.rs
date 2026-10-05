//! Source jumps and wiki/Markdown link navigation.

use super::*;

impl Workspace {
    pub(super) fn apply_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, offset)) = self.pending_jump.clone() else {
            return;
        };
        let Some(tab) = self
            .active
            .and_then(|i| self.tabs.get(i))
            .filter(|t| t.path == path)
        else {
            return;
        };
        let _ = tab;
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.jump(offset, window, cx));
        }
        self.pending_jump = None;
    }
    pub(super) fn follow_markdown_link(
        &mut self,
        from: PathBuf,
        href: &str,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if inkstone_core::rendering::is_external_link(href) {
            cx.open_url(href);
            return;
        }
        if let Some(vault) = &self.vault {
            let raw = href.split('#').next().unwrap_or("");
            if let Ok(decoded) = percent_encoding::percent_decode_str(raw).decode_utf8()
                && !decoded.contains(':')
                && !decoded.is_empty()
                && std::path::Path::new(decoded.as_ref())
                    .extension()
                    .is_some_and(|e| !e.eq_ignore_ascii_case("md"))
            {
                let reference = inkstone_core::rendering::Reference {
                    from: from.clone(),
                    target: href.into(),
                    wiki: false,
                };
                match inkstone_core::rendering::asset_path(
                    &vault.root,
                    &reference,
                    &self.index.files,
                ) {
                    Some(path) => {
                        cx.open_with_system(&path);
                        return;
                    }
                    _ => {
                        self.notifications
                            .publish("附件不存在或超出笔记库。".into());
                        cx.notify();
                        return;
                    }
                }
            }
        }
        let (resolution, heading) = self.index.resolve_markdown(&from, href);
        match resolution {
            Resolution::Found(path) => {
                if let Some(range) =
                    heading.and_then(|heading| self.index.anchor_range(&path, &heading))
                {
                    self.pending_jump = Some((path.clone(), range.start));
                }
                self.open_link_note(path, new_tab, window, cx);
                self.apply_jump(window, cx);
            }
            Resolution::Missing(path) => self.notifications.publish(format!(
                "本地 Markdown 链接目标不存在：{}。如需创建笔记，请使用双链。",
                path.display()
            )),
            _ => self
                .notifications
                .publish("此本地链接不是有效的库内 Markdown 目标。".into()),
        }
        cx.notify();
    }
    pub(super) fn follow_link(
        &mut self,
        from: PathBuf,
        target: String,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.index.resolve(&from, &target) {
            Resolution::Found(path) => {
                if let Some((_, heading)) = target.split_once('#')
                    && let Some(range) = self.index.anchor_range(&path, heading)
                {
                    self.pending_jump = Some((path.clone(), range.start));
                }
                self.open_link_note(path, new_tab, window, cx);
                self.apply_jump(window, cx);
            }
            Resolution::Missing(path) => {
                if self.tabs.iter().any(|t| t.path == path) {
                    self.open_link_note(path, new_tab, window, cx);
                } else {
                    if new_tab || self.current_view_pinned() {
                        self.views.secondary_focused = false;
                    }
                    self.add_tab(path, None, true, window, cx);
                    self.save_all(window, cx);
                }
            }
            Resolution::Ambiguous(paths) => {
                self.notifications.publish(format!(
                    "同名笔记有 {} 个，请使用目录/笔记名，或在搜索结果中选择。",
                    paths.len()
                ));
                self.search.update(cx, |state, cx| {
                    state.set_value(target.split('#').next().unwrap_or(""), window, cx)
                });
                self.fulltext = false;
                self.run_search(cx);
            }
            Resolution::Invalid => self
                .notifications
                .publish("双链路径无效或超出笔记库。".into()),
        }
        cx.notify();
    }
}
