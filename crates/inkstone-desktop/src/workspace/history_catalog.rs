//! Browse recorded history paths even when no corresponding note is open.
use super::focus_reveal::FocusReveal;
use super::*;
use gpui_component::{Disableable, button::Button};
use inkstone_core::vault::HistoryNote;

const PER_PAGE: usize = 5;
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
            .child("整库历史 · 包含已删除或外部重命名后的旧路径")
            .child(
                div().text_sm().whitespace_normal().child(
                    "按记录归属查找；外部重命名不会自动绑定新文件。恢复为副本，保留现有文件。",
                ),
            )
            .when(state.loading, |s| s.child("正在读取历史目录……"))
            .when(!state.message.is_empty(), |s| {
                s.child(state.message.clone())
            })
            .when(
                !state.loading && state.message.is_empty() && state.notes.is_empty(),
                |s| s.child("暂无历史记录"),
            )
            .when(pages > 0, |s| {
                s.child(
                    div()
                        .track_focus(&focus)
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(format!("历史目录：第 {} / {pages} 页", state.page + 1))
                        .children([false, true].into_iter().map(|next| {
                            let focus = focus.clone();
                            let id = ("history-catalog-page", usize::from(next));
                            FocusReveal::new(
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
                                        let last = state.notes.len().saturating_sub(1) / PER_PAGE;
                                        state.page = if next {
                                            state.page.saturating_add(1).min(last)
                                        } else {
                                            state.page.saturating_sub(1)
                                        };
                                        window.focus(&focus, cx);
                                        cx.notify();
                                    })),
                            )
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
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().whitespace_normal().child(format!(
                                "{} · {} 条记录 · {:.1} KiB",
                                path.display(),
                                note.records,
                                note.bytes as f64 / 1024.
                            )))
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
}
