//! Read-only, paginated inspection of valid descriptors without adjacent bodies.
use super::*;

fn text(id: impl Into<ElementId>, value: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Label)
        .aria_label(value.clone())
        .min_w_0()
        .whitespace_normal()
        .child(value)
}

impl Workspace {
    pub(super) fn missing_sync_payload_panel(&self, cx: &Context<Self>) -> AnyElement {
        let state = &self.ui.sync_recovery;
        let entries = &state.inventory.missing_payloads;
        let Some(vault) = self.vault.as_ref().filter(|_| !entries.is_empty()) else {
            return div().into_any_element();
        };
        let pages = entries.len().div_ceil(RECORDS_PER_PAGE);
        let page = state.missing_page.min(pages - 1);
        let focus = state
            .missing_focus
            .get_or_init(|| cx.focus_handle())
            .clone();
        div().flex().flex_col().gap_2().min_w_0()
            .child(text("sync-missing-help", format!(
                "{} 条描述记录的备份正文缺失（已计入不可读记录）。可能来自中断或外部修改；不能据此认定已安全删除。描述文件仍保留并计入占用，清理继续受保护。", entries.len())))
            .child(div().track_focus(&focus).flex().flex_wrap().gap_2()
                .child(text("sync-missing-page-label", format!("正文缺失记录：第 {} / {pages} 页", page + 1)))
                .children([false, true].into_iter().map(|next| {
                    let focus = focus.clone();
                    let id = ("sync-missing-page", usize::from(next));
                    div().debug_selector(move || format!("sync-missing-page-{next}"))
                        .child(FocusReveal::new((ElementId::from(id), "focus"), &self.ui.recovery_scroll,
                            Button::new(id).label(if next { "缺失记录下一页" } else { "缺失记录上一页" })
                                .disabled(if next { page + 1 >= pages } else { page == 0 })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let state = &mut this.ui.sync_recovery;
                                    let last = state.inventory.missing_payloads.len().saturating_sub(1) / RECORDS_PER_PAGE;
                                    state.missing_page = if next { state.missing_page.saturating_add(1).min(last) } else { state.missing_page.saturating_sub(1) };
                                    window.focus(&focus, cx);
                                    cx.notify();
                                }))))
                })))
            .children(entries.iter().enumerate().skip(page * RECORDS_PER_PAGE).take(RECORDS_PER_PAGE).map(|(i, entry)| {
                let description = format!("同步备份正文缺失 · 原路径：{} · 预期正文：{} · 描述文件：{} · 描述占用 {} 字节 · 时间：{}",
                    entry.original.display(), entry.backup.display(), entry.metadata.display(), entry.metadata_bytes,
                    chrono::DateTime::<chrono::Local>::from(entry.modified).format("%Y-%m-%d %H:%M:%S"));
                let path = vault.root.join(&entry.metadata);
                div().flex().flex_col().gap_1().min_w_0()
                    .debug_selector(move || format!("sync-missing-record-{i}"))
                    .child(text(("sync-missing-description", i), description.clone()))
                    .child(FocusReveal::new(("sync-missing-reveal-focus", i), &self.ui.recovery_scroll,
                        Button::new(("sync-missing-reveal", i)).label("定位描述文件")
                            .accessibility_label(format!("定位描述文件：{description}"))
                            .on_click(move |_, _, cx| cx.reveal_path(&path))))
            })).into_any_element()
    }
}
