use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::Selectable;
use gpui_component::button::*;

impl Workspace {
    fn default_hotkeys(id: usize) -> Vec<String> {
        #[cfg(target_os = "macos")]
        if id == 98 {
            return vec!["cmd-/".into()];
        }
        #[cfg(target_os = "macos")]
        if id == 96 {
            return vec!["ctrl-l".into()];
        }
        #[cfg(target_os = "macos")]
        if id == 35 {
            return vec!["cmd-l".into()];
        }
        #[cfg(target_os = "macos")]
        if id == 95 {
            return vec!["cmd-shift-k".into()];
        }
        #[cfg(target_os = "macos")]
        if (86..=94).contains(&id) {
            return vec![format!("cmd-{}", id - 85)];
        }
        #[cfg(target_os = "macos")]
        if id == 79 {
            return vec!["cmd-d".into()];
        }
        #[cfg(target_os = "macos")]
        if id == 80 {
            return vec!["cmd-shift-l".into()];
        }
        if id == 39 {
            return vec!["ctrl-p".into(), "ctrl-shift-p".into()];
        }
        commands::COMMANDS
            .iter()
            .find(|c| c.0 == id)
            .map(|c| {
                if c.2.is_empty() {
                    vec![]
                } else {
                    vec![c.2.to_lowercase().replace('+', "-")]
                }
            })
            .unwrap_or_default()
    }
    pub(super) fn hotkeys(&self, id: usize) -> Vec<String> {
        self.ui.prefs.hotkeys.get(&id).cloned().unwrap_or_else(|| {
            Self::default_hotkeys(id)
                .into_iter()
                .filter(|key| {
                    let Ok(stroke) = Keystroke::parse(key) else {
                        return false;
                    };
                    !self.ui.prefs.hotkeys.iter().any(|(other, keys)| {
                        *other != id
                            && commands::COMMANDS.iter().any(|c| c.0 == *other)
                            && keys.iter().any(|key| Self::key_matches(key, &stroke))
                    })
                })
                .collect()
        })
    }
    pub(super) fn hotkey_label(&self, id: usize) -> String {
        self.hotkeys(id)
            .iter()
            .map(|key| {
                key.split('-')
                    .map(|part| match part {
                        "ctrl" => "Ctrl".to_string(),
                        "shift" => "Shift".to_string(),
                        "alt" => "Alt".to_string(),
                        "cmd" => "Cmd".to_string(),
                        "super" => "Super".to_string(),
                        other => other.to_uppercase(),
                    })
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }
    fn key_matches(key: &str, stroke: &Keystroke) -> bool {
        Keystroke::parse(key).is_ok_and(|k| k.key == stroke.key && k.modifiers == stroke.modifiers)
    }
    fn assign_hotkey(&mut self, id: usize, key: &Keystroke) -> Result<(), String> {
        let function = key
            .key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n));
        if !(key.modifiers.control || key.modifiers.alt || key.modifiers.platform || function)
            || matches!(
                key.key.as_str(),
                "control" | "shift" | "alt" | "platform" | "function"
            )
        {
            return Err("请使用 Ctrl、Alt 组合键或 F1–F24，避免覆盖普通输入。".into());
        }
        if [
            "ctrl-a",
            "ctrl-c",
            "ctrl-x",
            "ctrl-v",
            "ctrl-z",
            "ctrl-y",
            "ctrl-enter",
            "ctrl-f",
        ]
        .iter()
        .any(|s| Self::key_matches(s, key))
            || (cfg!(target_os = "macos")
                && [
                    "cmd-a",
                    "cmd-c",
                    "cmd-x",
                    "cmd-v",
                    "cmd-z",
                    "cmd-shift-z",
                    "cmd-f",
                    "cmd-q",
                    "cmd-h",
                    "cmd-alt-h",
                    "cmd-m",
                    "ctrl-cmd-f",
                ]
                .iter()
                .any(|s| Self::key_matches(s, key)))
        {
            return Err("该组合用于系统菜单或编辑器的选择、剪贴板、撤销、查找及链接跳转。".into());
        }
        if let Some((_, title, _)) = commands::COMMANDS
            .iter()
            .find(|c| c.0 != id && self.hotkeys(c.0).iter().any(|s| Self::key_matches(s, key)))
        {
            return Err(format!("与“{title}”冲突，请先清除该命令的绑定。"));
        }
        let mut bindings = self.hotkeys(id);
        if !bindings.iter().any(|s| Self::key_matches(s, key)) {
            bindings.push(key.unparse());
        }
        self.ui.prefs.hotkeys.insert(id, bindings);
        Ok(())
    }
    fn reset_hotkeys(&mut self, id: usize) -> Result<(), String> {
        for key in Self::default_hotkeys(id) {
            if let Ok(stroke) = Keystroke::parse(&key)
                && let Some((_, title, _)) = commands::COMMANDS.iter().find(|c| {
                    c.0 != id
                        && self
                            .hotkeys(c.0)
                            .iter()
                            .any(|s| Self::key_matches(s, &stroke))
                })
            {
                return Err(format!("默认快捷键已被“{title}”使用，请先清除冲突绑定。"));
            }
        }
        self.ui.prefs.hotkeys.remove(&id);
        Ok(())
    }
    pub(super) fn handle_hotkey(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        if self.ui.link_update.is_some()
            || self
                .ui
                .inline_title
                .as_ref()
                .is_some_and(|edit| edit.input.read(cx).focus_handle(cx).is_focused(window))
            || self
                .current_pane()
                .is_some_and(|p| p.read(cx).has_footnote_editor())
        {
            if commands::COMMANDS.iter().any(|c| {
                self.hotkeys(c.0)
                    .iter()
                    .chain(Self::default_hotkeys(c.0).iter())
                    .any(|s| Self::key_matches(s, key))
            }) {
                cx.stop_propagation();
            }
            return;
        }
        if self.ui.settings
            && let Some(id) = self.ui.hotkey_recording
        {
            cx.stop_propagation();
            if key.key == "escape" {
                self.ui.hotkey_recording = None;
                self.ui.hotkey_message = "已取消录入".into();
            } else {
                match self.assign_hotkey(id, key) {
                    Ok(()) => {
                        self.ui.hotkey_recording = None;
                        self.ui.hotkey_message = "快捷键已更新".into();
                        self.persist_workspace(cx);
                    }
                    Err(message) => self.ui.hotkey_message = message,
                }
            }
            cx.notify();
            return;
        }
        if !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && !key.key.starts_with('f')
        {
            return;
        }
        if let Some(id) = commands::COMMANDS
            .iter()
            .find(|c| self.hotkeys(c.0).iter().any(|s| Self::key_matches(s, key)))
            .map(|c| c.0)
        {
            cx.stop_propagation();
            if matches!(id, 86..=94)
                && (self.ui.settings
                    || self.ui.name_mode.is_some()
                    || self.ui.property_open
                    || self.ui.trash_open)
            {
                return;
            }
            if matches!(id, 79..=84 | 95..=96 | 98)
                && !self.current_pane().is_some_and(|pane| {
                    let pane = pane.read(cx);
                    !pane.reading && pane.editor.read(cx).focus_handle(cx).is_focused(window)
                })
            {
                return;
            }
            self.execute_command(id, window, cx);
        } else if commands::COMMANDS.iter().any(|c| {
            Self::default_hotkeys(c.0)
                .iter()
                .any(|s| Self::key_matches(s, key))
        }) {
            // Suppress superseded static/component bindings as well.
            cx.stop_propagation();
        }
    }
    pub(super) fn settings_nav(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-navigation")
            .w(px(192.))
            .flex_shrink_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .p_3()
            .bg(self.side())
            .border_r_1()
            .border_color(self.border())
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_2()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .text_color(rgb(0x777777))
                    .child("选项"),
            )
            .children(
                [
                    (5, "外观", "palette"),
                    (6, "界面", "monitor"),
                    (0, "编辑器", "pencil"),
                    (2, "文件与链接", "folder"),
                    (7, "备份与恢复", "history"),
                    (1, "快捷键", "command"),
                ]
                .into_iter()
                .map(|(i, title, symbol)| {
                    Button::new(("settings-tab", i))
                        .ghost()
                        .h(px(36.))
                        .w_full()
                        .selected(self.ui.settings_tab == i)
                        .toggled(self.ui.settings_tab == i)
                        .when(self.ui.settings_tab == i, |button| {
                            button.bg(crate::theme::palette(self.ui.prefs.light).selected)
                        })
                        .accessibility_label(title)
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .child(ui::icon(symbol).size(px(16.)))
                                .child(title),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.ui.settings_tab = i;
                            this.ui.settings_scroll.set_offset(Point::default());
                            if i == 2 {
                                this.prepare_file_settings(w, cx);
                            }
                            this.ui.hotkey_recording = None;
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }
    pub(super) fn hotkey_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.ui.hotkey_filter.read(cx).value().to_lowercase();
        div()
            .flex()
            .gap_1()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                self.settings_content()
                    .gap_2()
                    .child(Input::new(&self.ui.hotkey_filter))
                    .child(div().text_size(px(MIN_UI_FONT_SIZE)).child(
                        if self.ui.hotkey_recording.is_some() {
                            "请按组合键，Esc 取消。".to_string()
                        } else {
                            self.ui.hotkey_message.clone()
                        },
                    ))
                    .child(
                        div()
                            .id("hotkey-list")
                            .overflow_y_scroll()
                            .max_h(px(430.))
                            .children(
                                commands::COMMANDS
                                    .iter()
                                    .filter(|(_, title, _)| title.to_lowercase().contains(&query))
                                    .map(|&(id, title, _)| {
                                        div()
                                            .flex()
                                            .flex_col()
                                            .p_2()
                                            .gap_1()
                                            .border_b_1()
                                            .border_color(self.border())
                                            .child(title)
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .text_size(px(MIN_UI_FONT_SIZE))
                                                            .child(self.hotkey_label(id)),
                                                    )
                                                    .child(
                                                        Button::new(("hotkey-add", id))
                                                            .compact()
                                                            .label("添加")
                                                            .on_click(cx.listener(
                                                                move |this, _, w, cx| {
                                                                    this.ui.hotkey_recording =
                                                                        Some(id);
                                                                    this.ui.hotkey_message.clear();
                                                                    w.focus(
                                                                        &this.ui.modal_focus,
                                                                        cx,
                                                                    );
                                                                    cx.notify();
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new(("hotkey-clear", id))
                                                            .compact()
                                                            .label("清除")
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.ui
                                                                        .prefs
                                                                        .hotkeys
                                                                        .insert(id, vec![]);
                                                                    this.ui.hotkey_recording = None;
                                                                    this.persist_workspace(cx);
                                                                    cx.notify();
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new(("hotkey-reset", id))
                                                            .compact()
                                                            .label("默认")
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.ui.hotkey_message =
                                                                        match this.reset_hotkeys(id)
                                                                        {
                                                                            Ok(()) => {
                                                                                "已恢复默认快捷键"
                                                                                    .into()
                                                                            }
                                                                            Err(error) => error,
                                                                        };
                                                                    this.ui.hotkey_recording = None;
                                                                    this.persist_workspace(cx);
                                                                    cx.notify();
                                                                },
                                                            )),
                                                    ),
                                            )
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PlatformKeys;
    use core::prelude::v1::test;
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn native_editing_and_application_keys_cannot_be_shadowed(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, _| {
                for key in ["cmd-v", "cmd-z", "cmd-shift-z", "cmd-q", "cmd-h", "cmd-m"] {
                    assert!(
                        w.assign_hotkey(12, &Keystroke::parse(key).unwrap())
                            .is_err(),
                        "{key}"
                    );
                }
                assert!(
                    w.assign_hotkey(12, &Keystroke::parse("cmd-alt-b").unwrap())
                        .is_ok()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn comment_hotkey_toggles_all_selections_and_can_be_reassigned(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab(
                    "comments.md".into(),
                    Some("word word".into()),
                    false,
                    window,
                    cx,
                );
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| {
                    s.set_selected_range(0..4, cx);
                    s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, window, cx);
                    s.focus(window, cx);
                });
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let key = if cfg!(target_os = "macos") {
            "cmd-/"
        } else {
            "ctrl-/"
        };
        visual.simulate_platform_keystrokes(key);
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), "%%word%% %%word%%");
            assert!(s.has_multiple_selections());
        });
        visual.simulate_platform_keystrokes(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        editor.read_with(&visual, |s, _| assert_eq!(s.value().as_ref(), "word word"));
        handle
            .update(&mut visual, |w, _, _| {
                w.ui.prefs.hotkeys.insert(98, vec![]);
                w.assign_hotkey(98, &Keystroke::parse("ctrl-alt-/").unwrap())
                    .unwrap();
            })
            .unwrap();
        visual.simulate_platform_keystrokes(key);
        editor.read_with(&visual, |s, _| assert_eq!(s.value().as_ref(), "word word"));
        visual.simulate_platform_keystrokes("ctrl-alt-/");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), "%%word%% %%word%%")
        });
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.settings = true;
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-alt-/");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), "%%word%% %%word%%")
        });
    }
    #[gpui::test]
    fn select_line_shortcut_handles_crlf_endpoints_and_empty_documents(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, selected, expected) in [
            ("甲\r\n乙\r\n丙", 3..3, 0..5),
            ("甲\r\n乙\r\n丙", 0..5, 0..5),
            ("甲\r\n乙\r\n丙", 3..10, 0..10),
            ("甲\r\n乙\r\n丙", 13..13, 10..13),
            ("", 0..0, 0..0),
        ] {
            let handle = cx.add_window(Workspace::new);
            let editor = handle
                .update(cx, |w, window, cx| {
                    w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                    let editor = w.current_pane().unwrap().read(cx).editor.clone();
                    editor.update(cx, |s, cx| s.set_selected_range(selected, cx));
                    editor
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_platform_keystrokes("alt-l");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), source);
                assert_eq!(s.selected_range(), expected);
            });
        }
    }

    #[gpui::test]
    fn select_line_shortcut_preserves_disjoint_blocks_for_editing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "a\nb\nc\nb\ne";
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(6..7, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-shift-l alt-l");
        handle
            .update(&mut visual, |_, window, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    assert_eq!(s.selected_range(), 2..4);
                    s.replace_text_in_range(None, "X", window, cx);
                    assert_eq!(s.value(), "a\nXc\nXe");
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert!(s.has_multiple_selections());
        });
        visual.simulate_platform_keystrokes("ctrl-shift-k");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "a\nc\ne"));
    }

    #[gpui::test]
    fn delete_line_shortcut_preserves_crlf_boundaries_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, selected, expected, caret) in [
            ("aa\r\naaaa\r\na", 1..1, "aaaa\r\na", Some(1..1)),
            ("aa\r\naaaa\r\na", 11..11, "aa\r\naaaa", Some(8..8)),
            ("aa\r\naaaa\r\na", 0..4, "aaaa\r\na", None),
            ("中文😀", 3..3, "", Some(0..0)),
            ("", 0..0, "", Some(0..0)),
            ("aa\nbb\ncc\ndd", 1..6, "cc\ndd", None),
        ] {
            let handle = cx.add_window(Workspace::new);
            let editor = handle
                .update(cx, |w, window, cx| {
                    w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                    let editor = w.current_pane().unwrap().read(cx).editor.clone();
                    editor.update(cx, |s, cx| s.set_selected_range(selected.clone(), cx));
                    editor
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_platform_keystrokes("ctrl-shift-k");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), expected);
                if let Some(caret) = &caret {
                    assert_eq!(s.selected_range(), *caret);
                }
            });
            visual.simulate_platform_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), source);
                assert_eq!(s.selected_range(), selected);
            });
        }
    }

    #[gpui::test]
    fn delete_line_shortcut_handles_disjoint_selections_and_wrapped_lines(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "a\nb\nc\nb\ne";
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(2..3, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-shift-l ctrl-shift-k");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "a\nc\ne"));
        visual.simulate_platform_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |_, window, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    assert_eq!(s.value(), source);
                    s.replace_text_in_range(None, "X", window, cx);
                    assert_eq!(s.value(), "a\nX\nc\nX\ne");
                    s.set_value(format!("{}\nnext\nlast", "中文😀 ".repeat(100)), window, cx);
                    s.set_selected_range(0..0, cx);
                });
            })
            .unwrap();
        visual.simulate_resize(size(px(500.), px(500.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-shift-k");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "next\nlast");
            assert!(s.selected_range().start <= 4);
        });
    }

    #[gpui::test]
    fn numbered_tab_shortcuts_follow_visible_order_and_custom_bindings(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-1 ctrl-9");
        handle
            .update(&mut visual, |w, window, cx| {
                assert!(w.active.is_none());
                for index in 0..10 {
                    w.add_tab(
                        format!("note{index}.md").into(),
                        Some(String::new()),
                        false,
                        window,
                        cx,
                    );
                }
            })
            .unwrap();
        for (key, index) in [("ctrl-1", 0), ("ctrl-8", 7), ("ctrl-9", 9)] {
            visual.simulate_platform_keystrokes(key);
            handle
                .update(&mut visual, |w, _, _| assert_eq!(w.active, Some(index)))
                .unwrap();
        }
        handle
            .update(&mut visual, |w, window, cx| {
                w.tabs.swap(0, 9);
                w.focus_primary(0, window, cx);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-9");
        handle
            .update(&mut visual, |w, window, cx| {
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("note0.md"));
                w.tabs.truncate(3);
                w.focus_primary(0, window, cx);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-8");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.active, Some(0)))
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-9");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.active, Some(2));
                w.ui.prefs.hotkeys.insert(86, vec!["ctrl-alt-1".into()]);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-1");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.active, Some(2)))
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-alt-1");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.active, Some(0)))
            .unwrap();
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.settings = true;
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-9");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.active, Some(0)))
            .unwrap();
    }

    #[gpui::test]
    fn line_copy_shortcuts_preserve_selection_and_multiple_carets(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let source = "甲\r\n乙";
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(8..8, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("shift-left alt-shift-down");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "甲\r\n乙\r\n乙");
            assert_eq!(s.selected_range(), 10..13);
        });
        visual.simulate_platform_keystrokes("shift-right");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 13..13));
        visual.simulate_platform_keystrokes("ctrl-z alt-shift-up");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "甲\r\n乙\r\n乙");
            assert_eq!(s.selected_range(), 5..8);
        });
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.update(&mut visual, |s, cx| s.set_selected_range(0..0, cx));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-alt-down alt-shift-down");
        handle
            .update(&mut visual, |_, window, cx| {
                editor.update(cx, |s, cx| {
                    assert_eq!(s.value(), "甲\r\n乙\r\n甲\r\n乙");
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "x", window, cx);
                    assert_eq!(s.value(), "甲\r\n乙\r\nx甲\r\nx乙");
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
    }

    #[gpui::test]
    fn line_move_shortcuts_preserve_crlf_direction_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let source = "甲\r\n乙\r\n丙";
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(8..8, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("shift-left alt-up");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "乙\r\n甲\r\n丙");
            assert_eq!(s.selected_range(), 0..3);
        });
        visual.simulate_platform_keystrokes("shift-right");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 3..3));
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 5..8);
        });
        visual.simulate_platform_keystrokes("alt-down");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "甲\r\n丙\r\n乙");
            assert_eq!(s.selected_range(), 10..13);
        });
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
        handle
            .update(&mut visual, |_, window, cx| {
                editor.update(cx, |s, cx| {
                    s.set_value("同\n同", window, cx);
                    s.set_selected_range(0..0, cx);
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("alt-down");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "同\n同");
            assert_eq!(s.selected_range(), 4..4);
        });
    }

    #[gpui::test]
    fn all_occurrences_default_and_custom_sidebar_binding_do_not_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some("cat cat".into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(0..3, cx));
                w.ui.prefs.left_open = true;
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-shift-l");
        editor.read_with(&visual, |s, _| assert!(s.has_multiple_selections()));
        handle
            .update(&mut visual, |w, _, _| {
                assert!(w.ui.prefs.left_open);
                w.ui.prefs
                    .hotkeys
                    .insert(12, vec![crate::test_support::keys("ctrl-shift-l")]);
                assert!(w.hotkeys(80).is_empty());
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-shift-l");
        handle
            .update(&mut visual, |w, _, _| assert!(!w.ui.prefs.left_open))
            .unwrap();
    }

    #[gpui::test]
    fn occurrence_hotkeys_are_customizable_and_only_target_the_focused_editor(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some("cat cat".into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(0..0, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-d");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.selected_range(), 0..3);
            assert!(
                !s.has_multiple_selections(),
                "one key must invoke the command once"
            );
        });
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.prefs.hotkeys.insert(79, vec![]);
                w.assign_hotkey(79, &Keystroke::parse("ctrl-alt-d").unwrap())
                    .unwrap();
                editor.update(cx, |s, cx| s.set_selected_range(0..0, cx));
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-d");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 0..0));
        visual.simulate_platform_keystrokes("ctrl-alt-d ctrl-alt-d");
        editor.read_with(&visual, |s, _| assert!(s.has_multiple_selections()));
        handle
            .update(&mut visual, |w, window, cx| {
                editor.update(cx, |s, cx| s.set_selected_range(0..0, cx));
                w.ui.settings = true;
                w.ui.settings_tab = 1;
                w.ui.hotkey_filter.update(cx, |s, cx| s.focus(window, cx));
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-alt-d");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 0..0));
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.settings = false;
                w.ui.prefs.hotkeys.insert(79, vec![]);
                editor.update(cx, |s, cx| s.focus(window, cx));
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-d ctrl-alt-d");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 0..0));
    }

    #[gpui::test]
    fn occurrence_palette_command_targets_the_last_focused_split(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some("cat cat".into()), false, window, cx);
                let main = w.current_pane().unwrap().read(cx).editor.clone();
                main.update(cx, |s, cx| s.set_selected_range(0..0, cx));
                w.split_active(false, window, cx);
                w.focus_secondary(cx);
                let secondary = w.current_pane().unwrap().read(cx).editor.clone();
                secondary.update(cx, |s, cx| s.set_selected_range(0..0, cx));
                w.execute_command(39, window, cx);
                assert!(w.command_open);
                w.execute_command(79, window, cx);
                assert!(!w.command_open);
                assert_eq!(secondary.read(cx).selected_range(), 0..3);
                assert_eq!(main.read(cx).selected_range(), 0..0);
                assert!(secondary.read(cx).focus_handle(cx).is_focused(window));
            })
            .unwrap();
    }

    #[gpui::test]
    fn link_shortcut_matches_markdown_default_and_respects_custom_wiki_binding(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some("中文😀".into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(0..10, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-k");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "[中文😀]()");
            assert_eq!(s.selected_range(), 13..13);
        });
        visual.simulate_platform_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.prefs.hotkeys.insert(26, vec!["ctrl-k".into()]);
                w.current_pane()
                    .unwrap()
                    .update(cx, |p, _| p.set_paths(Arc::new(vec!["/中文😀".into()])));
                assert!(w.hotkeys(59).is_empty());
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-k");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "[[中文😀]]");
            assert_eq!(s.selected_range(), 12..12);
            assert!(s.completion_menu_state().open);
        });
        visual.simulate_platform_keystrokes("down");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 12..12));
        visual.simulate_platform_keystrokes("enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "[[/中文😀]]"));
    }
    #[gpui::test]
    fn custom_hotkeys_replace_defaults_and_record_without_executing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-t");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.tabs.len(), 1);
                w.ui.prefs.hotkeys.insert(34, vec!["ctrl-alt-t".into()]);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-t");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 1))
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-alt-t");
        handle
            .update(&mut visual, |w, window, cx| {
                assert_eq!(w.tabs.len(), 2);
                w.ui.settings = true;
                w.ui.settings_tab = 1;
                w.ui.hotkey_recording = Some(4);
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-alt-t");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.tabs.len(), 2);
                assert!(w.ui.hotkey_message.contains("冲突"));
                assert_eq!(w.ui.hotkey_recording, Some(4));
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-alt-s");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.ui.hotkey_recording, None);
                assert!(w.hotkeys(4).contains(&"ctrl-alt-s".into()));
                let json = serde_json::to_string(&w.ui.prefs).unwrap();
                let restored: inkstone::preferences::Preferences =
                    serde_json::from_str(&json).unwrap();
                assert_eq!(restored.hotkeys, w.ui.prefs.hotkeys);
                w.ui.prefs.hotkeys.insert(4, vec![]);
                w.assign_hotkey(34, &Keystroke::parse("ctrl-s").unwrap())
                    .unwrap();
                assert!(w.reset_hotkeys(4).is_err());
                assert!(w.hotkeys(4).is_empty());
                assert!(
                    w.assign_hotkey(34, &Keystroke::parse("a").unwrap())
                        .is_err()
                );
                assert!(
                    w.assign_hotkey(34, &Keystroke::parse("ctrl-v").unwrap())
                        .is_err()
                );
                w.ui.hotkey_recording = Some(34);
            })
            .unwrap();
        visual.simulate_platform_keystrokes("escape");
        handle
            .update(&mut visual, |w, _, _| {
                assert!(w.ui.settings);
                assert!(w.ui.hotkey_recording.is_none());
            })
            .unwrap();
    }
}
