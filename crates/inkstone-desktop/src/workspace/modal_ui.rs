//! Modal container, keyboard navigation, pickers and property controls.
use super::ui::{icon, tool};
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_base::FocusTrapElement;
use gpui_component::date_picker::DatePicker;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Disableable, button::*};

impl Workspace {
    pub(super) fn modal_is_open(&self) -> bool {
        self.command_open
            || self.ui.quick_open
            || self.ui.property_open
            || self.ui.settings
            || self.agent.review.is_some()
            || self.ui.trash_open
            || self.ui.link_update.is_some()
    }

    pub(super) fn navigate_modal_tab(
        &self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        if key.key != "tab" || key.modifiers.control || key.modifiers.alt || key.modifiers.platform
        {
            return;
        }
        let advance = |window: &mut Window, cx: &mut App| {
            if key.modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
        };
        advance(window, cx);
        let first = window.focused(cx);
        while !self.ui.modal_focus.contains_focused(window, cx) {
            advance(window, cx);
            if window.focused(cx) == first {
                // A dialog without enabled tab stops still owns keyboard focus.
                window.focus(&self.ui.modal_focus, cx);
                break;
            }
        }
        cx.stop_propagation();
    }

    pub(super) fn modal(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let recovery = self.recovery_hub_is_open() || self.ui.history.is_some();
        let picker = self.command_open || self.ui.quick_open;
        let review = self.agent.review.is_some();
        let centered = self.ui.settings || review;
        let top = px(if recovery { 48. } else { 100. })
            .min(window.viewport_size().height * if picker { 0.12 } else { 0.15 })
            .max(px(16.));
        let available_height = if centered {
            (window.viewport_size().height - px(32.)).max(px(100.))
        } else {
            (window.viewport_size().height - top - px(16.)).max(px(100.))
        };
        if self.ui.link_update.is_some() {
            return div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(crate::theme::palette(self.ui.prefs.light).backdrop)
                .flex()
                .items_start()
                .justify_center()
                .pt(px(100.))
                .child(
                    div()
                        .id("link-update-dialog")
                        .track_focus(&self.ui.modal_focus)
                        .on_key_down(cx.listener(|this, event, window, cx| {
                            this.navigate_modal_tab(event, window, cx)
                        }))
                        .w(px(580.))
                        .p_5()
                        .gap_3()
                        .flex()
                        .flex_col()
                        .rounded(px(14.))
                        .bg(self.bg())
                        .border_1()
                        .border_color(self.border())
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("更新内部链接"),
                                )
                                .child(
                                    tool("close-links", "x", "不更新链接").on_click(
                                        cx.listener(|s, _, w, cx| s.close_overlays(w, cx)),
                                    ),
                                ),
                        )
                        .child(self.link_update_panel(cx))
                        .focus_trap("link-update-focus-trap", &self.ui.modal_focus),
                )
                .into_any_element();
        }
        let content = div()
            .id("modal-body")
            .debug_selector(|| "modal-body".into())
            .track_focus(&self.ui.modal_focus)
            .on_key_down(
                cx.listener(|this, event, window, cx| this.navigate_modal_tab(event, window, cx)),
            )
            .flex()
            .flex_col()
            .w(px(if self.ui.settings { 900. } else { 580. }))
            .when(self.recovery_hub_is_open(), |s| s.w(px(1000.)))
            .when(self.ui.history.is_some() || review, |s| s.w(px(1080.)))
            .when(self.ui.conflict_review.is_some(), |s| s.w(px(900.)))
            .when(picker, |s| s.w(px(700.)))
            .max_w((window.viewport_size().width - px(32.)).max(px(280.)))
            .max_h(px(if self.ui.settings { 700. } else { 650. }).min(available_height))
            .when(centered, |s| s.h(px(700.).min(available_height)))
            .when(recovery, |s| {
                s.h(px(700.).min(available_height)).max_h(available_height)
            })
            .p_5()
            .gap_3()
            .when(picker || self.ui.settings || recovery, |s| {
                s.p_0().gap_0().overflow_hidden()
            })
            .rounded(px(14.))
            .bg(self.bg())
            .border_1()
            .border_color(self.border())
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                if !this.command_open && !this.ui.quick_open {
                    return;
                }
                // Let focused result buttons handle their own Enter/Space action.
                if this.command_open && !this.ui.command.read(cx).focus_handle(cx).is_focused(w) {
                    return;
                }
                let input = if this.command_open {
                    &this.ui.command
                } else {
                    &this.search
                };
                if input.read(cx).is_composing() {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if key == "down" || key == "up" {
                    let n = if this.command_open {
                        this.filtered_commands(cx).len()
                    } else {
                        this.quick_result_count(cx)
                    };
                    if n > 0 {
                        this.ui.selected = if key == "down" {
                            (this.ui.selected + 1).min(n - 1)
                        } else {
                            this.ui.selected.saturating_sub(1)
                        };
                    }
                    this.ui.modal_scroll.scroll_to_item(this.ui.selected);
                    cx.stop_propagation();
                    cx.notify();
                } else if key == "enter" {
                    if this.command_open {
                        if let Some((id, _, _)) =
                            this.filtered_commands(cx).get(this.ui.selected).copied()
                        {
                            this.execute_command(id, w, cx);
                        }
                    } else {
                        this.submit_search_result(w, cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb_1()
                    .when(picker || self.ui.history.is_some(), |s| s.hidden())
                    .when(recovery, |s| {
                        s.px(px(24.))
                            .py(px(12.))
                            .flex_shrink_0()
                            .border_b_1()
                            .border_color(self.border())
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(self.ui.settings, |s| {
                        s.h(px(50.))
                            .flex_shrink_0()
                            .px_5()
                            .border_b_1()
                            .border_color(self.border())
                    })
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if self.command_open {
                                "命令面板"
                            } else if self.ui.quick_open {
                                "快速切换"
                            } else if self.ui.property_open {
                                "编辑属性"
                            } else if self.ui.settings {
                                "设置"
                            } else if review {
                                "审阅完整修改"
                            } else if self.ui.conflict_review.is_some() {
                                "比较并处理外部修改"
                            } else if self.ui.attachment_manager.is_some() {
                                "附件管理"
                            } else if self.ui.link_health.is_some() {
                                "链接健康检查"
                            } else if self.ui.bulk_edit.is_some() {
                                "批量修改预览"
                            } else if self.ui.table_editor.is_some() {
                                "可视化表格编辑"
                            } else if self.ui.history.is_some() {
                                if self.reviewing_draft() {
                                    "草稿恢复与比较"
                                } else {
                                    "笔记版本历史"
                                }
                            } else if self.ui.trash_open {
                                "文件恢复中心"
                            } else {
                                "操作"
                            }),
                    )
                    .child(
                        tool("close-modal", "x", "关闭 Esc")
                            .on_click(cx.listener(|this, _, w, cx| this.close_overlays(w, cx))),
                    ),
            )
            .when(self.command_open, |s| {
                let commands = self.filtered_commands(cx);
                s.child(self.picker_search(true, cx)).child(
                    div()
                        .id("commands-list")
                        .track_scroll(&self.ui.modal_scroll)
                        .overflow_y_scroll()
                        .min_h_0()
                        .p_3()
                        .when(commands.is_empty(), |list| {
                            list.child(self.empty_state(
                                "command",
                                "未找到命令",
                                "试试搜索功能名称，例如「设置」或「分屏」。",
                            ))
                        })
                        .max_h(px(384.).min((available_height - px(80.)).max(px(40.))))
                        .children(commands.into_iter().enumerate().map(|(i, (id, label, _))| {
                            let colors = crate::theme::palette(self.ui.prefs.light);
                            let shortcut = self.hotkey_label(id);
                            let active = self.ui.selected == i;
                            Button::new(("command", id))
                                .ghost()
                                .w_full()
                                .accessibility_label(format!("{label} {shortcut}"))
                                .flex()
                                .items_center()
                                .justify_between()
                                .h(px(40.))
                                .px_3()
                                .gap_3()
                                .rounded(px(8.))
                                .cursor_pointer()
                                .when(active, |s| s.bg(colors.selected))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                                        .child(label),
                                )
                                .when(!shortcut.is_empty(), |s| {
                                    s.child(
                                        div()
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(6.))
                                            .bg(colors.surface)
                                            .border_1()
                                            .border_color(colors.border)
                                            .text_color(colors.muted)
                                            .text_size(px(12.5))
                                            .child(shortcut),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.execute_command(id, w, cx)
                                }))
                        })),
                )
            })
            .when(self.ui.quick_open, |s| {
                let result_height =
                    (self.quick_result_count(cx).max(3) as f32 * 38. + 16.).min(360.);
                s.child(self.picker_search(false, cx)).child(
                    div()
                        .flex()
                        .flex_col()
                        .min_h_0()
                        .overflow_hidden()
                        .h(px(result_height).min((available_height - px(80.)).max(px(40.))))
                        .child(self.search_list(true, cx)),
                )
            })
            .when(picker, |s| {
                let colors = crate::theme::palette(self.ui.prefs.light);
                s.child(
                    div()
                        .flex_shrink_0()
                        .h(px(34.))
                        .border_t_1()
                        .border_color(colors.border)
                        .bg(colors.subtle_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.5))
                        .text_color(colors.muted)
                        .child(if self.command_open {
                            "↑↓ 导航  ·  ↵ 执行  ·  Esc 退出"
                        } else {
                            "↑↓ 导航  ·  ↵ 打开  ·  Esc 退出"
                        }),
                )
            })
            .when(self.ui.property_open, |s| {
                s.child(Input::new(&self.ui.property_key))
                    .child(self.property_type_control(cx))
                    .when(
                        matches!(
                            self.effective_property_kind(cx),
                            inkstone_core::properties::Kind::Date
                                | inkstone_core::properties::Kind::DateTime
                        ),
                        |s| {
                            let i = usize::from(
                                self.effective_property_kind(cx)
                                    == inkstone_core::properties::Kind::DateTime,
                            );
                            s.child(
                                DatePicker::new(&self.ui.property_dates[i])
                                    .placeholder("选择日期")
                                    .cleanable(true),
                            )
                        },
                    )
                    .when(
                        !matches!(
                            self.effective_property_kind(cx),
                            inkstone_core::properties::Kind::Checkbox
                                | inkstone_core::properties::Kind::List
                        ),
                        |s| s.child(Input::new(&self.ui.property_value)),
                    )
                    .when(
                        self.effective_property_kind(cx) == inkstone_core::properties::Kind::List,
                        |s| s.child(self.property_list_control(cx)),
                    )
                    .when(!self.ui.property_error.is_empty(), |s| {
                        s.child(
                            div()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .text_color(crate::theme::palette(self.ui.prefs.light).danger)
                                .whitespace_normal()
                                .child(self.ui.property_error.clone()),
                        )
                    })
                    .child(
                        Button::new("save-property")
                            .primary()
                            .label("保存属性")
                            .on_click(cx.listener(|this, _, w, cx| this.save_property(w, cx))),
                    )
                    .when(self.ui.property_original.is_some(), |s| {
                        s.child(
                            Button::new("delete-property").label("删除属性").on_click(
                                cx.listener(|this, _, w, cx| this.delete_property(w, cx)),
                            ),
                        )
                    })
            })
            .when(self.ui.settings, |s| s.child(self.settings_panel(cx)))
            .when(self.ui.bulk_edit.is_some(), |s| {
                s.child(self.bulk_edit_panel(cx))
            })
            .when(review, |s| s.child(self.proposal_review_panel(cx)))
            .when(self.ui.table_editor.is_some(), |s| {
                s.child(self.table_editor_panel(cx))
            })
            .when(self.ui.link_health.is_some(), |s| {
                s.child(self.link_health_panel(cx))
            })
            .when(self.ui.attachment_manager.is_some(), |s| {
                s.child(self.attachments_panel(cx))
            })
            .when(self.ui.history.is_some(), |s| {
                s.child(self.history_panel(window, cx))
            })
            .when(self.ui.conflict_review.is_some(), |s| {
                s.child(self.conflict_panel(cx))
            })
            .when(self.recovery_hub_is_open(), |s| {
                s.child(self.recovery_hub_panel(window, cx))
            });

        div()
            .absolute()
            .inset_0()
            .bg(crate::theme::palette(self.ui.prefs.light).backdrop)
            .occlude()
            .flex()
            .justify_center()
            .when(centered, |s| s.items_center())
            .when(!centered, |s| s.items_start().pt(top))
            .child(content.focus_trap("workspace-modal-focus-trap", &self.ui.modal_focus))
            .into_any_element()
    }
    fn picker_search(&self, command: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(px(54.))
            .px_4()
            .gap_2p5()
            .border_b_1()
            .border_color(colors.border)
            .child(
                icon(if command { "command" } else { "search" })
                    .size(px(17.))
                    .flex_shrink_0()
                    .text_color(colors.muted),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(if command {
                        &self.ui.command
                    } else {
                        &self.search
                    })
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
                ),
            )
            .child(
                tool("close-picker", "x", "关闭 Esc")
                    .on_click(cx.listener(|this, _, w, cx| this.close_overlays(w, cx))),
            )
            .into_any_element()
    }

    fn property_list_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let items = match self.property_list_values(cx) {
            Ok(items) => items,
            Err(error) => {
                return div()
                    .child(Input::new(&self.ui.property_value))
                    .child(div().text_size(px(MIN_UI_FONT_SIZE)).child(error))
                    .into_any_element();
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("property-list-items")
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(items.into_iter().enumerate().map(|(i, value)| {
                        let colors = crate::theme::palette(self.ui.prefs.light);
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2p5()
                            .py_0p5()
                            .rounded_full()
                            .bg(colors.accent_tint)
                            .border_1()
                            .border_color(colors.border)
                            .child(
                                div()
                                    .max_w(px(400.))
                                    .whitespace_normal()
                                    .child(value.clone()),
                            )
                            .child(
                                Button::new(("remove-property-item", i))
                                    .accessibility_label(format!("移除列表项目 {value}"))
                                    .ghost()
                                    .compact()
                                    .icon(icon("x"))
                                    .tooltip("移除此项目")
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.remove_property_list_item(i, w, cx)
                                    })),
                            )
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.ui.property_list_entry)),
                    )
                    .child(
                        Button::new("add-property-item")
                            .label("添加")
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.add_property_list_item(w, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
    fn property_type_control(&self, cx: &mut Context<Self>) -> AnyElement {
        use inkstone_core::properties::Kind;
        let reserved = matches!(
            self.ui.property_key.read(cx).value().as_ref(),
            "tags" | "aliases" | "cssclasses"
        );
        let kind = self.effective_property_kind(cx);
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .items_center()
            .gap_3()
            .child("属性类型")
            .child(
                Button::new("property-type")
                    .label(kind.label())
                    .disabled(reserved)
                    .dropdown_menu(move |mut menu, _, _| {
                        for choice in [
                            Kind::Text,
                            Kind::List,
                            Kind::Number,
                            Kind::Checkbox,
                            Kind::Date,
                            Kind::DateTime,
                        ] {
                            let weak = weak.clone();
                            menu = menu.item(
                                PopupMenuItem::new(choice.label())
                                    .checked(kind == choice)
                                    .on_click(move |_, w, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.change_property_kind(choice, w, cx);
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .when(kind == Kind::Checkbox, |s| {
                s.child(
                    super::settings_ui::setting_switch("property-checkbox")
                        .accessibility_label(format!(
                            "属性 {}",
                            self.ui.property_key.read(cx).value()
                        ))
                        .checked(self.ui.property_value.read(cx).value().as_ref() == "true")
                        .on_click(cx.listener(|this, checked: &bool, w, cx| {
                            this.ui.property_error.clear();
                            this.ui
                                .property_value
                                .update(cx, |s, cx| s.set_value(checked.to_string(), w, cx));
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use inkstone_core::vault::{RecoverySummary, TrashEntry};

    #[gpui::test]
    fn recovery_controls_scroll_into_view_in_both_keyboard_directions(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.new_blank(window, cx);
                w.ui.trash_open = true;
                w.ui.trash = (0..12)
                    .map(|i| TrashEntry {
                        stored: PathBuf::from(format!("ui-only-trash/{i}")),
                        original: PathBuf::from(format!(
                            "多层目录/较长的回收站原始笔记路径/第{i}篇笔记.md"
                        )),
                        directory: false,
                    })
                    .collect();
                w.recoveries = (0..12)
                    .map(|i| RecoverySummary {
                        journal: PathBuf::from(format!("ui-only-drafts/{i}")),
                        relative: PathBuf::from(format!(
                            "多层目录/较长的未保存草稿原始路径/第{i}篇笔记.md"
                        )),
                        modified: std::time::UNIX_EPOCH,
                        bytes: 123,
                    })
                    .collect();
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        let selectors = [
            "recovery-trash-0",
            "recovery-trash-1",
            "recovery-trash-2",
            "recovery-trash-3",
            "recovery-trash-4",
            "recovery-trash-5",
            "recovery-trash-6",
            "recovery-trash-7",
            "recovery-trash-8",
            "recovery-trash-9",
            "recovery-trash-10",
            "recovery-trash-11",
            "recovery-draft-0",
            "recovery-draft-1",
            "recovery-draft-2",
            "recovery-draft-3",
            "recovery-draft-4",
            "recovery-draft-5",
            "recovery-draft-6",
            "recovery-draft-7",
            "recovery-draft-8",
            "recovery-draft-9",
            "recovery-draft-10",
            "recovery-draft-11",
        ];
        for rem in [16., 24.] {
            visual.update(|w, _| w.set_rem_size(px(rem)));
            for source in [
                super::super::recovery_ui::Source::Trash,
                super::super::recovery_ui::Source::Drafts,
            ] {
                handle
                    .update(&mut visual, |w, _, _| {
                        w.ui.recovery_source = source;
                    })
                    .unwrap();
                for key in ["tab", "shift-tab"] {
                    handle
                        .update(&mut visual, |w, window, cx| {
                            window.focus(&w.ui.modal_focus, cx)
                        })
                        .unwrap();
                    let mut reached = std::collections::BTreeSet::new();
                    let mut focused_controls = Vec::new();
                    for _ in 0..40 {
                        let keystroke = Keystroke::parse(key).unwrap();
                        visual.simulate_event(KeyDownEvent {
                            keystroke: keystroke.clone(),
                            is_held: false,
                            prefer_character_input: false,
                        });
                        visual.simulate_event(KeyUpEvent { keystroke });
                        for _ in 0..2 {
                            visual.update(|w, cx| w.draw(cx).clear(cx));
                        }
                        if let Some(target) = visual.debug_bounds("focus-revealed-control") {
                            handle
                                .update(&mut visual, |w, window, cx| {
                                    let viewport = w.ui.recovery_scroll.bounds();
                                    assert!(
                                        viewport.top() >= px(0.)
                                            && viewport.bottom() <= window.viewport_size().height
                                    );
                                    assert!(
                                        target.top() >= viewport.top()
                                            && target.bottom() <= viewport.bottom(),
                                        "{key}, rem={rem}: {target:?} outside {viewport:?}"
                                    );
                                    let focus = window.focused(cx).unwrap();
                                    if !focused_controls.contains(&focus) {
                                        focused_controls.push(focus);
                                    }
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
                    assert_eq!(reached.len(), 12, "{key}, rem={rem}: {reached:?}");
                    assert_eq!(
                        focused_controls.len(),
                        12,
                        "all controls in the selected source"
                    );
                    handle
                        .update(&mut visual, |w, _, _| {
                            assert_eq!(w.ui.trash.len(), 12);
                            assert_eq!(w.recoveries.len(), 12);
                            assert_eq!(w.file_writes.pending(), 0);
                            assert!(!w.file_writes.operation_active());
                            assert!(w.ui.history.is_none());
                            assert!(w.recoveries.iter().all(|e| e.bytes == 123));
                        })
                        .unwrap();
                }
            }
        }
    }

    #[gpui::test]
    fn settings_modal_is_centered_across_viewport_sizes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.execute_command(14, window, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for (width, height) in [(1400., 960.), (1200., 800.), (1200., 620.), (800., 800.)] {
            visual.simulate_resize(size(px(width), px(height)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let bounds = visual
                .debug_bounds("modal-body")
                .expect("settings modal should render");
            let left = f32::from(bounds.left());
            let right = width - f32::from(bounds.right());
            let top = f32::from(bounds.top());
            let bottom = height - f32::from(bounds.bottom());
            assert!(
                (left - right).abs() <= 1.0,
                "viewport {width}x{height}: horizontal margins differ ({left} vs {right})"
            );
            assert!(
                (top - bottom).abs() <= 1.0,
                "viewport {width}x{height}: vertical margins differ ({top} vs {bottom})"
            );
            assert!(
                left >= 15.0 && right >= 15.0 && top >= 15.0 && bottom >= 15.0,
                "viewport {width}x{height}: margins too small (l={left}, r={right}, t={top}, b={bottom})"
            );
        }
    }
}
