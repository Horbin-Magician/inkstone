use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::button::*;

impl Workspace {
    pub(super) fn loading_workspace(&self) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        div()
            .id("workspace-loading")
            .debug_selector(|| "workspace-loading".into())
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .w(px(250.))
                    .flex_shrink_0()
                    .bg(colors.sidebar)
                    .border_r_1()
                    .border_color(colors.border)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children([0.7, 0.9, 0.6].into_iter().map(|width| {
                        div()
                            .h(px(12.))
                            .w(relative(width))
                            .rounded(px(4.))
                            .bg(colors.surface)
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .text_color(colors.muted)
                    .child("正在打开笔记库…"),
            )
            .into_any_element()
    }

    pub(super) fn welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        div()
            .id("welcome")
            .debug_selector(|| "welcome".into())
            .flex_1().min_h_0().flex().flex_col().items_center().justify_center()
            .px_6().pb(px(64.))
            .child(div().w(px(360.)).max_w_full().flex().flex_col().gap_3()
                .child(div().flex().items_center().gap_3().mb_3()
                    .child(div().size(px(42.)).rounded(px(12.)).bg(colors.selected)
                        .flex().items_center().justify_center()
                        .child(ui::icon("square-pen").size(px(22.)).text_color(colors.accent)))
                    .child(div().text_size(px(26.)).font_weight(FontWeight::SEMIBOLD).child(crate::product::name())))
                .child(div().text_size(px(18.)).font_weight(FontWeight::MEDIUM).child("你的笔记，从这里开始"))
                .child(div().text_size(px(MIN_UI_FONT_SIZE)).line_height(relative(1.7)).text_color(colors.muted)
                    .child("选择一个文件夹作为笔记库。笔记以 Markdown 文件保存在本地，随时可以打开和整理。"))
                .when(!self.status.is_empty(), |s| s.child(div()
                    .id("welcome-error").debug_selector(|| "welcome-error".into())
                    .text_size(px(MIN_UI_FONT_SIZE)).text_color(colors.muted)
                    .child(format!("无法打开笔记库：{}", self.status))))
                .child(Button::new("welcome-open-vault").primary().mt_4().h(px(42.)).w_full()
                    .label("打开笔记库").icon(ui::icon("folder-open"))
                    .on_click(cx.listener(|this, _, window, cx| this.choose_vault(window, cx))))
                .child(div().text_size(px(MIN_UI_FONT_SIZE)).text_center().text_color(colors.muted)
                    .child("可以选择已有笔记文件夹，也可以选择一个空文件夹")))
            .into_any_element()
    }
    pub(super) fn blank_note(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        div()
            .id("blank-note")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px_4()
            .pb(px(48.))
            .child(
                div()
                    .w(px(280.))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .mb_1()
                            .child("新标签页"),
                    )
                    .child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(colors.muted)
                            .mb_4()
                            .child("创建笔记，或继续之前的记录。"),
                    )
                    .children(
                        [
                            ("empty-new", "创建新笔记", "file-plus", 0),
                            ("empty-open", "查找笔记", "search", 2),
                        ]
                        .into_iter()
                        .map(|(id, label, symbol, command)| {
                            Button::new(id)
                                .ghost()
                                .w_full()
                                .h(px(40.))
                                .accessibility_label(label)
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            ui::icon(symbol).size(px(17.)).text_color(colors.muted),
                                        )
                                        .child(div().flex_1().child(label))
                                        .child(
                                            div()
                                                .text_size(px(MIN_UI_FONT_SIZE))
                                                .text_color(colors.muted)
                                                .child(self.hotkey_label(command)),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.execute_command(command, w, cx)
                                }))
                        }),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::Workspace;
    use crate::test_support::PlatformKeys;
    use gpui::{TestAppContext, VisualTestContext};

    #[gpui::test]
    fn welcome_hides_workspace_chrome_but_keeps_settings_accessible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("welcome").is_some());
        assert!(visual.debug_bounds("workspace-right-panel").is_none());
        assert!(visual.debug_bounds("workspace-status-bar").is_none());
        visual.simulate_platform_keystrokes("ctrl-,");
        handle
            .update(&mut visual, |workspace, window, cx| {
                assert!(workspace.ui.settings);
                workspace.close_overlays(window, cx);
                assert!(workspace.ui.prefs.left_open);
                assert!(workspace.ui.prefs.right_open);
            })
            .unwrap();
    }

    #[gpui::test]
    fn initial_lookup_shows_loading_then_welcome_when_no_recent_vault(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("workspace-loading").is_some());
        assert!(visual.debug_bounds("welcome").is_none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("workspace-loading").is_none());
        assert!(visual.debug_bounds("welcome").is_some());
    }

    #[gpui::test]
    fn failed_startup_returns_to_welcome_with_visible_error(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-missing-startup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.finish_startup(Some(root), window, cx);
                assert!(workspace.loading);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("welcome").is_none());
        assert!(visual.debug_bounds("workspace-loading").is_some());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("workspace-loading").is_none());
        assert!(visual.debug_bounds("welcome").is_some());
        assert!(visual.debug_bounds("welcome-error").is_some());
    }

    fn assert_startup_opens_without_welcome(cx: &mut TestAppContext, manual: bool) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-startup-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "# Restored").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                if manual {
                    workspace.load_vault(root.clone(), window, cx);
                    workspace.finish_startup(Some(root.join("missing")), window, cx);
                } else {
                    workspace.finish_startup(Some(root.clone()), window, cx);
                }
                assert_eq!(workspace.generation, 1);
                assert!(!workspace.startup_pending);
                assert!(workspace.loading);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("welcome").is_none());
        assert!(visual.debug_bounds("workspace-loading").is_some());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("welcome").is_none());
        assert!(visual.debug_bounds("workspace-loading").is_none());
        assert!(visual.debug_bounds("workspace-status-bar").is_some());
        handle
            .update(&mut visual, |workspace, _, _| {
                assert_eq!(workspace.tabs.len(), 1);
                assert_eq!(workspace.tabs[0].path, std::path::Path::new("note.md"));
                workspace.watcher = None;
                workspace.watch_events = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn automatic_restore_enters_workspace_without_welcome_flash(cx: &mut TestAppContext) {
        assert_startup_opens_without_welcome(cx, false);
    }

    #[gpui::test]
    fn manual_open_supersedes_pending_startup_without_welcome_flash(cx: &mut TestAppContext) {
        assert_startup_opens_without_welcome(cx, true);
    }
}
