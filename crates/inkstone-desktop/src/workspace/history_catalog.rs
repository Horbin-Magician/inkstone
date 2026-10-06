//! Browse recorded history paths even when no corresponding note is open.
use super::focus_reveal::FocusReveal;
use super::*;
use gpui_component::{Disableable, button::Button};
use inkstone_core::vault::HistoryNote;

const PER_PAGE: usize = 5;

fn catalog_text(id: impl Into<ElementId>, value: impl Into<String>) -> gpui::Stateful<gpui::Div> {
    let value = value.into();
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(value.clone())
        .min_w_0()
        .whitespace_normal()
        .child(value)
}

#[derive(Default)]
pub(super) struct State {
    notes: Vec<HistoryNote>,
    page: usize,
    loading: bool,
    message: String,
    pager_focus: std::cell::OnceCell<FocusHandle>,
}
impl Workspace {
    pub(super) fn refresh_history_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.ui.history_catalog = State {
            loading: true,
            ..Default::default()
        };
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx
            .background_executor()
            .spawn(async move { vault.history_notes() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let state = &mut this.ui.history_catalog;
                state.loading = false;
                match result {
                    Ok(notes) => state.notes = notes,
                    Err(error) => state.message = format!("无法读取整库历史：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn history_catalog_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.history_catalog;
        let pages = state.notes.len().div_ceil(PER_PAGE);
        let focus = state.pager_focus.get_or_init(|| cx.focus_handle()).clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(catalog_text(
                "history-catalog-title",
                "整库历史 · 包含已删除或外部重命名后的旧路径",
            ))
            .child(
                catalog_text(
                    "history-catalog-help",
                    "按记录归属查找；外部重命名不会自动绑定新文件。恢复为副本，保留现有文件。",
                )
                .text_sm(),
            )
            .when(state.loading, |s| {
                s.child(catalog_text(
                    "history-catalog-loading",
                    "正在读取历史目录……",
                ))
            })
            .when(!state.message.is_empty(), |s| {
                s.child(catalog_text(
                    "history-catalog-message",
                    state.message.clone(),
                ))
            })
            .when(
                !state.loading && state.message.is_empty() && state.notes.is_empty(),
                |s| s.child(catalog_text("history-catalog-empty", "暂无历史记录")),
            )
            .when(pages > 0, |s| {
                s.child(
                    div()
                        .track_focus(&focus)
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(catalog_text(
                            "history-catalog-page-status",
                            format!("历史目录：第 {} / {pages} 页", state.page + 1),
                        ))
                        .children([false, true].into_iter().map(|next| {
                            let focus = focus.clone();
                            let id = ("history-catalog-page", usize::from(next));
                            div()
                                .debug_selector(move || format!("history-catalog-page-{next}"))
                                .child(FocusReveal::new(
                                    (ElementId::from(id), "focus"),
                                    &self.ui.recovery_scroll,
                                    Button::new(id)
                                        .label(if next {
                                            "历史目录下一页"
                                        } else {
                                            "历史目录上一页"
                                        })
                                        .disabled(if next {
                                            state.page + 1 >= pages
                                        } else {
                                            state.page == 0
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let state = &mut this.ui.history_catalog;
                                            let last =
                                                state.notes.len().saturating_sub(1) / PER_PAGE;
                                            state.page = if next {
                                                state.page.saturating_add(1).min(last)
                                            } else {
                                                state.page.saturating_sub(1)
                                            };
                                            window.focus(&focus, cx);
                                            cx.notify();
                                        })),
                                ))
                        })),
                )
            })
            .children(
                state
                    .notes
                    .iter()
                    .enumerate()
                    .skip(state.page * PER_PAGE)
                    .take(PER_PAGE)
                    .map(|(i, note)| {
                        let path = note.relative.clone();
                        let id = ("history-catalog-note", i);
                        div()
                            .debug_selector(move || format!("history-catalog-row-{i}"))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(catalog_text(
                                ("history-catalog-summary", i),
                                format!(
                                    "{} · {} 条记录 · {} 字节（{:.1} KiB）",
                                    path.display(),
                                    note.records,
                                    note.bytes,
                                    note.bytes as f64 / 1024.
                                ),
                            ))
                            .child(catalog_text(
                                ("history-catalog-modified", i),
                                format!(
                                    "最近记录文件更新时间：{}",
                                    chrono::DateTime::<chrono::Local>::from(note.modified)
                                        .format("%Y-%m-%d %H:%M:%S")
                                ),
                            ))
                            .child(FocusReveal::new(
                                (ElementId::from(id), "focus"),
                                &self.ui.recovery_scroll,
                                Button::new(id)
                                    .label(format!("查看历史：{}", path.display()))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_history_path(path.clone(), window, cx)
                                    })),
                            ))
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn history_catalog_requests_are_invalidated_by_close_and_library_switch(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-catalog-ui-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for i in 0..12 {
            vault
                .save(&PathBuf::from(format!("{i:02}.md")), None, "历史")
                .unwrap();
        }
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.execute_command(16, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.ui.history_catalog.notes.len(), 12);
                assert!(!w.ui.history_catalog.loading);
                w.refresh_trash(cx);
                w.close_overlays(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(w.ui.history_catalog.notes.is_empty());
                w.refresh_history_catalog(cx);
                w.generation += 1;
                w.ui.history_catalog = Default::default();
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| assert!(w.ui.history_catalog.notes.is_empty()))
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn history_catalog_pages_keep_long_paths_reachable_at_minimum_size(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-catalog-keys-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.ui.trash_open = true;
                w.ui.history_catalog.notes = (0..12)
                    .map(|i| HistoryNote {
                        relative: PathBuf::from(format!(
                            "{i:02}-旧目录/{}/{}/笔记👩‍💻.md",
                            "long-segment-".repeat(12),
                            "长目录".repeat(16)
                        )),
                        records: 1,
                        bytes: 1024,
                        modified: std::time::SystemTime::UNIX_EPOCH,
                    })
                    .collect();
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        fn key(v: &mut VisualTestContext, key: &str) {
            let keystroke = Keystroke::parse(key).unwrap();
            v.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            v.simulate_event(KeyUpEvent { keystroke });
            for _ in 0..2 {
                v.update(|w, cx| w.draw(cx).clear(cx));
            }
        }
        let selectors = [
            "history-catalog-row-0",
            "history-catalog-row-1",
            "history-catalog-row-2",
            "history-catalog-row-3",
            "history-catalog-row-4",
            "history-catalog-row-5",
            "history-catalog-row-6",
            "history-catalog-row-7",
            "history-catalog-row-8",
            "history-catalog-row-9",
            "history-catalog-row-10",
            "history-catalog-row-11",
        ];
        for rem in [16., 24.] {
            visual.update(|w, _| w.set_rem_size(px(rem)));
            handle
                .update(&mut visual, |w, window, cx| {
                    w.ui.history_catalog.page = 0;
                    window.focus(&w.ui.modal_focus, cx);
                })
                .unwrap();
            for (step, page) in [0, 1, 2, 1, 0].into_iter().enumerate() {
                let direction = if step < 2 { "tab" } else { "shift-tab" };
                let mut reached = std::collections::BTreeSet::new();
                for _ in 0..24 {
                    key(&mut visual, direction);
                    if let Some(target) = visual.debug_bounds("focus-revealed-control") {
                        handle
                            .update(&mut visual, |w, _, _| {
                                let viewport = w.ui.recovery_scroll.bounds();
                                assert!(
                                    target.top() >= viewport.top()
                                        && target.bottom() <= viewport.bottom()
                                        && target.left() >= viewport.left()
                                        && target.right() <= viewport.right(),
                                    "rem={rem}, page={page}: {target:?} outside {viewport:?}"
                                );
                            })
                            .unwrap();
                        for (i, selector) in selectors.iter().enumerate() {
                            if let Some(row) = visual.debug_bounds(selector)
                                && target.top() >= row.top()
                                && target.bottom() <= row.bottom()
                            {
                                reached.insert(i);
                            }
                        }
                    }
                }
                let start = page * PER_PAGE;
                let end = (start + PER_PAGE).min(12);
                assert_eq!(reached, (start..end).collect(), "rem={rem}, page={page}");
                for (i, selector) in selectors.iter().enumerate() {
                    assert_eq!(
                        visual.debug_bounds(selector).is_some(),
                        (start..end).contains(&i)
                    );
                }
                handle
                    .update(&mut visual, |w, _, _| {
                        assert_eq!(w.ui.history_catalog.page, page);
                        assert!(w.ui.history.is_none());
                        assert_eq!(w.file_writes.pending(), 0);
                    })
                    .unwrap();
                if step == 4 {
                    break;
                }
                let selector = if step < 2 {
                    "history-catalog-page-true"
                } else {
                    "history-catalog-page-false"
                };
                let mut found = false;
                for _ in 0..24 {
                    key(&mut visual, direction);
                    if let (Some(target), Some(button)) = (
                        visual.debug_bounds("focus-revealed-control"),
                        visual.debug_bounds(selector),
                    ) && target.top() >= button.top()
                        && target.bottom() <= button.bottom()
                        && target.left() >= button.left()
                        && target.right() <= button.right()
                    {
                        found = true;
                        break;
                    }
                }
                assert!(found);
                key(&mut visual, "enter");
                handle
                    .update(&mut visual, |w, window, cx| {
                        assert!(
                            w.ui.history_catalog
                                .pager_focus
                                .get()
                                .unwrap()
                                .contains_focused(window, cx)
                        );
                    })
                    .unwrap();
            }
        }
        assert!(vault.scan_files().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
