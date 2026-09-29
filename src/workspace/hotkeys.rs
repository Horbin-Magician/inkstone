use super::*;
use gpui_component::button::*;

impl Workspace {
    fn default_hotkeys(id: usize) -> Vec<String> {
        if id == 39 {
            return vec!["ctrl-p".into(), "ctrl-shift-p".into()];
        }
        ui::COMMANDS
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
                            && ui::COMMANDS.iter().any(|c| c.0 == *other)
                            && keys.iter().any(|key| Self::key_matches(key, &stroke))
                    })
                })
                .collect()
        })
    }
    pub(super) fn hotkey_label(&self, id: usize) -> String {
        self.hotkeys(id)
            .iter()
            .map(|key| key.replace('-', "+"))
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
        {
            return Err("该组合用于编辑器的选择、剪贴板、撤销、查找或链接跳转。".into());
        }
        if let Some((_, title, _)) = ui::COMMANDS
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
                && let Some((_, title, _)) = ui::COMMANDS.iter().find(|c| {
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
            if ui::COMMANDS.iter().any(|c| {
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
        if let Some(id) = ui::COMMANDS
            .iter()
            .find(|c| self.hotkeys(c.0).iter().any(|s| Self::key_matches(s, key)))
            .map(|c| c.0)
        {
            cx.stop_propagation();
            self.execute_command(id, window, cx);
        } else if ui::COMMANDS.iter().any(|c| {
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
            .w(px(140.))
            .p_3()
            .bg(self.side())
            .rounded(px(6.))
            .flex()
            .flex_col()
            .gap_2()
            .children(
                ["编辑器与外观", "快捷键", "文件与链接", "日记"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, title)| {
                        Button::new(("settings-tab", i))
                            .ghost()
                            .label(title)
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.ui.settings_tab = i;
                                if i == 2 {
                                    this.prepare_file_settings(w, cx);
                                }
                                if i == 3 {
                                    this.prepare_daily_settings(w, cx);
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
            .gap_4()
            .min_h(px(330.))
            .child(self.settings_nav(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.ui.hotkey_filter))
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(if self.ui.hotkey_recording.is_some() {
                                "请按组合键，Esc 取消。".to_string()
                            } else {
                                self.ui.hotkey_message.clone()
                            }),
                    )
                    .child(
                        div()
                            .id("hotkey-list")
                            .overflow_y_scroll()
                            .max_h(px(430.))
                            .children(
                                ui::COMMANDS
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
                                                            .text_size(px(12.))
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
    use core::prelude::v1::test;
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
        visual.simulate_keystrokes("ctrl-k");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "[中文😀]()");
            assert_eq!(s.selected_range(), 13..13);
        });
        visual.simulate_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.prefs.hotkeys.insert(26, vec!["ctrl-k".into()]);
                w.current_pane()
                    .unwrap()
                    .update(cx, |p, _| p.set_paths(Arc::new(vec!["/中文😀".into()])));
                assert!(w.hotkeys(59).is_empty());
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-k");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "[[中文😀]]");
            assert_eq!(s.selected_range(), 12..12);
            assert!(s.completion_menu_state().open);
        });
        visual.simulate_keystrokes("down");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 12..12));
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "[[/中文😀]]"));
    }
    #[gpui::test]
    fn custom_hotkeys_replace_defaults_and_record_without_executing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-t");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.tabs.len(), 1);
                w.ui.prefs.hotkeys.insert(34, vec!["ctrl-alt-t".into()]);
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-t");
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 1))
            .unwrap();
        visual.simulate_keystrokes("ctrl-alt-t");
        handle
            .update(&mut visual, |w, window, cx| {
                assert_eq!(w.tabs.len(), 2);
                w.ui.settings = true;
                w.ui.settings_tab = 1;
                w.ui.hotkey_recording = Some(4);
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-alt-t");
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.tabs.len(), 2);
                assert!(w.ui.hotkey_message.contains("冲突"));
                assert_eq!(w.ui.hotkey_recording, Some(4));
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-alt-s");
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
        visual.simulate_keystrokes("escape");
        handle
            .update(&mut visual, |w, _, _| {
                assert!(w.ui.settings);
                assert!(w.ui.hotkey_recording.is_none());
            })
            .unwrap();
    }
}
