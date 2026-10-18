//! Source jumps and wiki/Markdown link navigation.

use super::*;

impl Workspace {
    fn link_anchor_offset(
        &mut self,
        path: &std::path::Path,
        fragment: &str,
        wiki: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.flush_document_views(window, cx);
        let anchor = if wiki {
            inkstone_core::index::anchor_range
        } else {
            inkstone_core::index::anchor_range_decoded
        };
        let range = if let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) {
            let text = tab.save.text_snapshot(cx);
            anchor(&text, &inkstone_core::index::parse(&text), fragment)
        } else {
            self.index
                .notes
                .get(path)
                .and_then(|note| anchor(&note.text, &note.parsed, fragment))
        };
        range.map(|range| range.start)
    }

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
                self.pending_jump = heading
                    .and_then(|heading| self.link_anchor_offset(&path, &heading, false, window, cx))
                    .map(|offset| (path.clone(), offset));
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
                self.pending_jump = target
                    .split_once('#')
                    .and_then(|(_, heading)| {
                        self.link_anchor_offset(&path, heading, true, window, cx)
                    })
                    .map(|offset| (path.clone(), offset));
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn newer_link_request_clears_pending_anchor_for_the_same_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-link-anchor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let handle = cx.add_window(Workspace::new);
        for (round, (markdown, suffix)) in [
            (false, ""),
            (false, "#Missing"),
            (true, ""),
            (true, "#Missing"),
        ]
        .into_iter()
        .enumerate()
        {
            let path = PathBuf::from(format!("target-{round}.md"));
            let text = "intro\n\n# Target\n\nbody\n";
            std::fs::write(vault.root.join(&path), text).unwrap();
            handle
                .update(cx, |w, window, cx| {
                    w.vault = Some(vault.clone());
                    Arc::make_mut(&mut w.index).update(path.clone(), text.into());
                    w.add_tab("source.md".into(), Some("source".into()), false, window, cx);
                    let first = format!("{}#Target", path.display());
                    let second = format!("{}{suffix}", path.display());
                    if markdown {
                        w.follow_markdown_link("source.md".into(), &first, false, window, cx);
                        assert!(w.pending_jump.is_some());
                        w.follow_markdown_link("source.md".into(), &second, false, window, cx);
                    } else {
                        w.follow_link("source.md".into(), first, false, window, cx);
                        assert!(w.pending_jump.is_some());
                        w.follow_link("source.md".into(), second, false, window, cx);
                    }
                    assert!(
                        w.pending_jump.is_none(),
                        "new request must not inherit a stale heading"
                    );
                })
                .unwrap();
            cx.run_until_parked();
            handle
                .update(cx, |w, _, cx| {
                    assert_eq!(w.tabs[w.active.unwrap()].path, path);
                    assert_eq!(
                        w.current_pane()
                            .unwrap()
                            .read(cx)
                            .editor
                            .read(cx)
                            .selected_range()
                            .start,
                        0
                    );
                })
                .unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn markdown_percent_encoded_heading_is_decoded_once(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let source = "# A\nwrong\n\n# %41\ncorrect\n";
                Arc::make_mut(&mut w.index).update("a.md".into(), source.into());
                assert_eq!(
                    w.link_anchor_offset(std::path::Path::new("a.md"), "%41", false, window, cx),
                    source.find("# %41")
                );
                w.add_tab("a.md".into(), Some(source.into()), false, window, cx);
                w.follow_markdown_link("a.md".into(), "#%2541", false, window, cx);
                assert_eq!(
                    w.current_pane()
                        .unwrap()
                        .read(cx)
                        .editor
                        .read(cx)
                        .selected_range()
                        .start,
                    source.find("# %41").unwrap()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn link_anchors_use_pending_duplicate_and_split_edits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let source = "# Target\n\nText ^block\n";
                Arc::make_mut(&mut w.index).update("a.md".into(), source.into());
                w.add_tab("a.md".into(), Some(source.into()), false, window, cx);
                w.add_tab("a.md".into(), None, false, window, cx);
                for markdown in [false, true] {
                    if markdown {
                        w.split_active(true, window, cx);
                    }
                    let draft = format!(
                        "新增内容 {}\n\n{source}",
                        if markdown { "分屏" } else { "第二视图" }
                    );
                    let pane = w.current_pane().unwrap();
                    pane.read(cx)
                        .editor
                        .clone()
                        .update(cx, |s, cx| s.replace_all(draft.clone(), window, cx));
                    if markdown {
                        w.follow_markdown_link("a.md".into(), "#%5Eblock", false, window, cx);
                    } else {
                        w.follow_link("a.md".into(), "a#Target".into(), false, window, cx);
                    }
                    assert_eq!(w.current_pane().unwrap().entity_id(), pane.entity_id());
                    assert_eq!(
                        pane.read(cx).editor.read(cx).selected_range().start,
                        draft
                            .find(if markdown { "Text" } else { "# Target" })
                            .unwrap()
                    );
                }
            })
            .unwrap();
    }
}
