use super::*;
use gpui_component::button::*;

impl Workspace {
    pub(super) fn offer_link_updates(
        &mut self,
        edits: LinkEdits,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if edits.is_empty() {
            return;
        }
        if self.ui.prefs.always_update_links {
            self.apply_link_updates(edits, window, cx);
        } else {
            self.ui.link_update = Some(edits);
            self.ui.link_update_scroll = UniformListScrollHandle::new();
            self.ui.more = false;
            window.focus(&self.ui.modal_focus, cx);
            cx.notify();
        }
    }
    pub(super) fn confirm_link_updates(
        &mut self,
        always: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edits) = self.ui.link_update.take() else {
            return;
        };
        self.focus_after_link_update(window, cx);
        if always {
            self.ui.prefs.always_update_links = true;
            self.persist_workspace(cx);
        }
        self.apply_link_updates(edits, window, cx);
    }
    fn apply_link_updates(
        &mut self,
        edits: LinkEdits,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            let mut written = vec![];
            let mut errors = vec![];
            for (path, before, after) in edits {
                match vault.save(&path, Some(&before), &after) {
                    Ok(receipt) => written.push((path, before, receipt.text)),
                    Err(error) => errors.push(format!("{}：{error}", path.display())),
                }
            }
            (written, errors)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (written, errors) = task.await;
            let _ = this.update_in(cx, |this, w, cx| {
                this.ui.file_operation = false;
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.status = format!("已更新 {} 篇笔记的内部链接。", written.len());
                let mut changed = vec![];
                for (path, before, after) in written {
                    let Some(id) = this.tabs.iter().find(|t| t.path == path).map(|t| t.id) else {
                        continue;
                    };
                    let composing = this.has_pending_input(id, w, cx);
                    let tab = this.tabs.iter_mut().find(|t| t.id == id).unwrap();
                    let editor = tab.pane.read(cx).editor.clone();
                    if tab.dirty
                        || composing
                        || tab.baseline.as_deref() != Some(&before)
                        || editor.read(cx).value().as_ref() != before
                    {
                        tab.conflict = true;
                        tab.dirty = true;
                        this.status
                            .push_str(&format!(" {} 的新编辑已保留，请处理冲突。", path.display()));
                    } else {
                        tab.baseline = Some(after.clone());
                        editor.update(cx, |s, cx| {
                            let selected = s.selected_range();
                            let scroll = s.scroll_offset();
                            s.set_value(after, w, cx);
                            s.set_selected_range(selected, cx);
                            s.set_scroll_offset(scroll, cx);
                        });
                        changed.push(id);
                    }
                }
                for id in changed {
                    this.sync_to_split(id, w, cx);
                }
                if !errors.is_empty() {
                    this.ui.window_close_requested = false;
                    this.status
                        .push_str(&format!(" 部分链接未更新：{}", errors.join("；")));
                }
                this.sync_reference_contexts(cx);
                this.rescan = true;
                this.refresh_requested = true;
                this.tick(w, cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn link_update_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let edits = self.ui.link_update.as_ref().unwrap();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(format!(
                "文件已重命名或移动。是否更新以下 {} 篇笔记中的内部链接？",
                edits.len()
            ))
            .child(
                uniform_list(
                    "link-update-files",
                    edits.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                        range
                            .filter_map(|i| {
                                this.ui.link_update.as_ref()?.get(i).map(|(path, _, _)| {
                                    div()
                                        .h(px(28.))
                                        .truncate()
                                        .child(path.to_string_lossy().to_string())
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.ui.link_update_scroll)
                .h(px(edits.len().min(8) as f32 * 28.))
                .w_full(),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("links-skip")
                            .label("不更新")
                            .on_click(cx.listener(|s, _, w, cx| {
                                s.close_overlays(w, cx);
                            })),
                    )
                    .child(
                        Button::new("links-once").label("仅本次更新").on_click(
                            cx.listener(|s, _, w, cx| s.confirm_link_updates(false, w, cx)),
                        ),
                    )
                    .child(
                        Button::new("links-always").label("始终更新").on_click(
                            cx.listener(|s, _, w, cx| s.confirm_link_updates(true, w, cx)),
                        ),
                    ),
            )
            .into_any_element()
    }
    pub(super) fn focus_after_link_update(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.ui.property_open {
            self.ui.property_value.update(cx, |s, cx| s.focus(w, cx));
        } else if self.ui.name_mode.is_some() {
            self.name.update(cx, |s, cx| s.focus(w, cx));
        } else if self.command_open {
            self.ui.command.update(cx, |s, cx| s.focus(w, cx));
        } else if self.ui.quick_open {
            self.search.update(cx, |s, cx| s.focus(w, cx));
        } else if self.ui.settings || self.ui.trash_open {
            w.focus(&self.ui.modal_focus, cx);
        } else if let Some(p) = self.current_pane() {
            p.update(cx, |p, cx| p.focus_view(w, cx));
        } else {
            w.focus(&self.ui.workspace_focus, cx);
        }
    }
}
