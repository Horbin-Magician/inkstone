use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::{Disableable, button::*};

impl Workspace {
    fn create_first_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_some() || self.loading || self.ui.pending_file_writes > 0 {
            return;
        }
        let generation = self.generation;
        let start = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let dialog = cx.prompt_for_new_path(&start, Some("我的笔记库"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(root))) = dialog.await else {
                return;
            };
            let ready = this
                .update(cx, |this, cx| {
                    if this.generation != generation
                        || this.vault.is_some()
                        || this.loading
                        || this.ui.pending_file_writes > 0
                    {
                        return false;
                    }
                    this.ui.pending_file_writes += 1;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !ready {
                return;
            }
            let directory = root.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    // Never merge into or overwrite an existing directory.
                    std::fs::create_dir(directory)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => this.load_vault(root, window, cx),
                    Err(error) => {
                        this.notifications.publish(format!(
                            "创建笔记库失败：{error}。请选择尚不存在的文件夹名称。"
                        ));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

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
            .flex_1().min_h_0().overflow_y_scroll().flex().flex_col().items_center()
            .px_6().py_6()
            .child(div().w(px(440.)).max_w_full().flex_shrink_0().flex().flex_col().gap_3()
                .child(div().flex().items_center().gap_3().mb_3()
                    .child(div().size(px(42.)).rounded(px(12.)).bg(colors.selected)
                        .flex().items_center().justify_center()
                        .child(ui::icon("square-pen").size(px(22.)).text_color(colors.accent)))
                    .child(div().text_size(px(26.)).font_weight(FontWeight::SEMIBOLD).child(crate::product::name())))
                .child(div().text_size(px(18.)).font_weight(FontWeight::MEDIUM).child("你的笔记，从这里开始"))
                .child(div().text_size(px(MIN_UI_FONT_SIZE)).line_height(relative(1.7)).text_color(colors.muted)
                    .child("选择一个文件夹作为笔记库。笔记以 Markdown 文件保存在本地，随时可以打开和整理。"))
                .when(!self.notifications.text().is_empty(), |s| s.child(div()
                    .id("welcome-error").debug_selector(|| "welcome-error".into())
                    .text_size(px(MIN_UI_FONT_SIZE)).text_color(colors.muted)
                    .child(self.notifications.text().to_owned())))
                .child(Button::new("welcome-create-vault").primary().mt_4().h(px(42.)).w_full()
                    .label("创建新笔记库").accessibility_label("创建新笔记库")
                    .disabled(self.ui.pending_file_writes > 0)
                    .on_click(cx.listener(|this, _, window, cx| this.create_first_vault(window, cx))))
                .child(Button::new("welcome-open-vault").h(px(42.)).w_full()
                    .label("打开已有笔记库").accessibility_label("打开已有笔记库").disabled(self.ui.pending_file_writes > 0).icon(ui::icon("folder-open"))
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
            .overflow_y_scroll()
            .px_4()
            .py_4()
            .child(
                div()
                    .w(px(440.))
                    .max_w_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .mb_1()
                            .child(if self.index.notes.is_empty() { "写下第一篇笔记" } else { "新标签页" }),
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
                            ("empty-recovery", "文件恢复", "history", 16),
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
                    )
                    .child(div().id("first-note-guide").debug_selector(|| "first-note-guide".into())
                        .text_size(px(MIN_UI_FONT_SIZE)).text_color(colors.muted).line_height(relative(1.6))
                        .flex().flex_col().gap_2()
                        .child(format!("创建笔记后即可写作。{}", if self.hotkey_label(4).is_empty() {
                            "通过文件菜单中的“保存当前更改”保存 Markdown 文件。".to_owned()
                        } else { format!("按 {} 保存 Markdown 文件。", self.hotkey_label(4)) }))
                        .child("顶部模式菜单：实时预览可边写边看排版；源码模式显示 Markdown 标记；阅读视图用于浏览，不修改正文。")
                        .child("未保存的编辑会另存为恢复草稿，不会定时改写原文件。意外退出后，可从“文件恢复”比较草稿并恢复为副本；最近尚未持久化的输入仍可能丢失。")),
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
    fn create_first_vault_cancels_rejects_existing_and_opens_new(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-onboarding-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("keep.md"), b"existing").unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.create_first_vault(window, cx))
            .unwrap();
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.vault.is_none());
                assert_eq!(w.ui.pending_file_writes, 0);
                w.create_first_vault(window, cx);
            })
            .unwrap();
        cx.simulate_new_path_selection(|_| Some(root.clone()));
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.vault.is_none());
                assert!(w.notifications.text().contains("创建笔记库失败"));
                assert_eq!(w.ui.pending_file_writes, 0);
                w.create_first_vault(window, cx);
            })
            .unwrap();
        cx.simulate_new_path_selection(|_| Some(root.join("new")));
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.vault.as_ref().unwrap().root,
                    std::fs::canonicalize(root.join("new")).unwrap()
                );
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(w.index.notes.is_empty());
                w.execute_command(16, window, cx);
                assert!(w.ui.trash_open);
                w.watcher = None;
                w.watch_events = None;
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read(root.join("keep.md")).unwrap(), b"existing");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn closing_recovery_on_blank_tab_restores_visible_workspace_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.new_blank(window, cx);
                w.execute_command(16, window, cx);
                w.close_overlays(window, cx);
                assert!(w.ui.workspace_focus.is_focused(window));
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        handle
            .update(&mut visual, |_, window, cx| {
                assert!(window.is_action_available(&super::super::NewNote, cx));
                assert!(window.is_action_available(&super::super::OpenVault, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn welcome_tab_order_reaches_creation_and_existing_vault_dialogs(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        // Header settings, then creation, then existing vault.
        for key in "tab tab enter".split_whitespace() {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            visual.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(gpui::KeyUpEvent { keystroke });
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(
            visual.did_prompt_for_new_path(),
            "creation must be keyboard reachable; existing dialog={}",
            visual.did_prompt_for_paths()
        );
        visual.simulate_new_path_selection(|_| None);
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, window, cx| {
                window.focus(&w.ui.workspace_focus, cx)
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        for key in "tab tab tab enter".split_whitespace() {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            visual.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(gpui::KeyUpEvent { keystroke });
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(
            visual.did_prompt_for_paths(),
            "existing vault must be keyboard reachable"
        );
        visual.simulate_path_prompt_response(|_| None);
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, window, cx| {
                window.focus(&w.ui.workspace_focus, cx)
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        for key in "tab tab tab shift-tab enter".split_whitespace() {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            visual.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(gpui::KeyUpEvent { keystroke });
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(
            visual.did_prompt_for_new_path(),
            "reverse traversal must return to creation"
        );
        visual.simulate_new_path_selection(|_| None);
        visual.run_until_parked();
    }

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
