//! Modal container, keyboard navigation, pickers and property controls.
use super::ui::{NameMode, icon, tool};
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::date_picker::DatePicker;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Disableable, button::*};

impl Workspace {
    fn navigate_modal_tab(
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
        let picker = self.command_open || self.ui.quick_open;
        let top = px(100.)
            .min(window.viewport_size().height * if picker { 0.12 } else { 0.15 })
            .max(px(16.));
        let available_height = (window.viewport_size().height - top - px(16.)).max(px(100.));
        if self.ui.link_update.is_some() {
            return div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(rgba(0x00000066))
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
                        .p_3()
                        .gap_3()
                        .flex()
                        .flex_col()
                        .rounded(px(12.))
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
                                .child("更新内部链接")
                                .child(
                                    tool("close-links", "x", "不更新链接").on_click(
                                        cx.listener(|s, _, w, cx| s.close_overlays(w, cx)),
                                    ),
                                ),
                        )
                        .child(self.link_update_panel(cx)),
                )
                .into_any_element();
        }
        let content = div()
            .id("modal-body")
            .track_focus(&self.ui.modal_focus)
            .on_key_down(
                cx.listener(|this, event, window, cx| this.navigate_modal_tab(event, window, cx)),
            )
            .flex()
            .flex_col()
            .w(px(if self.ui.settings { 900. } else { 580. }))
            .when(self.ui.history.is_some(), |s| s.w(px(900.)))
            .when(self.ui.conflict_review.is_some(), |s| s.w(px(900.)))
            .when(picker, |s| s.w(px(700.)))
            .max_w((window.viewport_size().width - px(32.)).max(px(280.)))
            .max_h(px(if self.ui.settings { 700. } else { 650. }).min(available_height))
            .when(self.ui.settings, |s| s.h(px(700.).min(available_height)))
            .p_3()
            .gap_2()
            .when(picker || self.ui.settings, |s| {
                s.p_0().gap_0().overflow_hidden()
            })
            .rounded(px(12.))
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
                let key = event.keystroke.key.as_str();
                let n = if this.command_open {
                    this.filtered_commands(cx).len()
                } else {
                    this.visible_search_hits(true, cx).len()
                };
                if key == "down" || key == "up" {
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
                        this.open_selected_result(w, cx);
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
                    .when(picker, |s| s.hidden())
                    .when(self.ui.settings, |s| {
                        s.h(px(48.))
                            .flex_shrink_0()
                            .px_3()
                            .border_b_1()
                            .border_color(self.border())
                    })
                    .child(if self.command_open {
                        "命令面板"
                    } else if self.ui.quick_open {
                        "快速切换"
                    } else if self.ui.property_open {
                        "编辑属性"
                    } else if self.ui.settings {
                        "设置"
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
                        "文件恢复"
                    } else {
                        match self.ui.name_mode {
                            Some(NameMode::Rename) => "重命名或移动文件",
                            Some(NameMode::Folder) => "新建文件夹",
                            Some(NameMode::RenameFolder) => "重命名文件夹",
                            _ => "新建笔记",
                        }
                    })
                    .child(
                        tool("close-modal", "x", "关闭 Esc")
                            .on_click(cx.listener(|this, _, w, cx| this.close_overlays(w, cx))),
                    ),
            )
            .when(self.command_open, |s| {
                s.child(self.picker_search(true, cx)).child(
                    div()
                        .id("commands-list")
                        .track_scroll(&self.ui.modal_scroll)
                        .overflow_y_scroll()
                        .min_h_0()
                        .p_3()
                        .when(self.filtered_commands(cx).is_empty(), |list| {
                            list.child(self.empty_state(
                                "command",
                                "未找到命令",
                                "试试搜索功能名称，例如「设置」或「分屏」。",
                            ))
                        })
                        .max_h(px(384.).min((available_height - px(80.)).max(px(40.))))
                        .children(self.filtered_commands(cx).into_iter().enumerate().map(
                            |(i, (id, label, _))| {
                                Button::new(("command", id))
                                    .ghost()
                                    .w_full()
                                    .accessibility_label(format!(
                                        "{label} {}",
                                        self.hotkey_label(id)
                                    ))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(38.))
                                    .px_3()
                                    .gap_3()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(self.ui.selected == i, |s| {
                                        s.bg(crate::theme::palette(self.ui.prefs.light).selected)
                                    })
                                    .child(div().flex_1().min_w_0().truncate().child(label))
                                    .child(
                                        div()
                                            .text_color(
                                                crate::theme::palette(self.ui.prefs.light).muted,
                                            )
                                            .text_size(px(MIN_UI_FONT_SIZE))
                                            .flex_shrink_0()
                                            .child(self.hotkey_label(id)),
                                    )
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.execute_command(id, w, cx)
                                    }))
                            },
                        )),
                )
            })
            .when(self.ui.quick_open, |s| {
                let result_height =
                    (self.visible_search_hits(true, cx).len().max(3) as f32 * 38. + 16.).min(360.);
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
                s.child(
                    div()
                        .flex_shrink_0()
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(MIN_UI_FONT_SIZE))
                        .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                        .child(if self.command_open {
                            "↑↓ 导航　↵ 使用　esc 退出"
                        } else {
                            "↑↓ 导航　↵ 打开　esc 退出"
                        }),
                )
            })
            .when(self.ui.name_mode.is_some(), |s| {
                s.child(Input::new(&self.name))
                    .child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                            .child("使用 / 指定文件夹路径，Enter 确认"),
                    )
                    .child(
                        Button::new("submit-name")
                            .primary()
                            .label("确认")
                            .on_click(cx.listener(|this, _, w, cx| this.submit_name(w, cx))),
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
                                .text_color(rgb(0xe87979))
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
                s.child(self.history_panel(cx))
            })
            .when(self.ui.conflict_review.is_some(), |s| {
                s.child(self.conflict_panel(cx))
            })
            .when(
                self.ui.trash_open
                    && self.ui.bulk_edit.is_none()
                    && self.ui.table_editor.is_none()
                    && self.ui.link_health.is_none()
                    && self.ui.attachment_manager.is_none()
                    && self.ui.history.is_none()
                    && self.ui.conflict_review.is_none(),
                |s| {
                    s.child(
                        div()
                            .id("trash-items")
                            .track_scroll(&self.ui.recovery_scroll)
                            .max_h(px(450.))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .p_2()
                                    .child(
                                        Button::new("recovery-refresh")
                                            .label("刷新恢复记录")
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.refresh_trash(cx)
                                                }),
                                            ),
                                    )
                                    .child(
                                        Button::new("recovery-history")
                                            .label("当前笔记版本历史")
                                            .disabled(self.active.is_none())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_history(window, cx)
                                            })),
                                    ),
                            )
                            .child(self.sync_recovery_panel(cx))
                            .child(
                                div()
                                    .p_2()
                                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                    .child("回收站 · 不自动清理 · 原路径存在时拒绝覆盖"),
                            )
                            .when(self.ui.trash.is_empty(), |s| {
                                s.child(div().p_2().child("回收站为空"))
                            })
                            .children(self.ui.trash.iter().enumerate().map(|(i, e)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .p_2()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div().truncate().child(
                                                    e.original.to_string_lossy().to_string(),
                                                ),
                                            )
                                            .child(div().text_sm().child(
                                                super::recovery::trash_details(
                                                    self.ui.trash_metadata.get(&e.stored),
                                                ),
                                            )),
                                    )
                                    .child(
                                        Button::new(("restore-trash", i))
                                            .compact()
                                            .label("恢复")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.restore_deleted(i, cx)
                                            })),
                                    )
                            }))
                            .child(
                                div()
                                    .p_2()
                                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                    .child("未保存草稿 · 点击比较、恢复或放弃"),
                            )
                            .children(self.recoveries.iter().enumerate().map(|(i, e)| {
                                let time: chrono::DateTime<chrono::Local> = e.modified.into();
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .p_2()
                                    .child(
                                        div()
                                            .truncate()
                                            .child(e.record.relative.to_string_lossy().to_string()),
                                    )
                                    .child(div().text_sm().child(format!(
                                        "草稿 · {} · {:.1} KiB · 恢复或放弃前保留",
                                        time.format("%Y-%m-%d %H:%M:%S"),
                                        e.bytes as f64 / 1024.
                                    )))
                                    .child(
                                        Button::new(("restore-draft", i))
                                            .ghost()
                                            .label("比较、恢复或放弃")
                                            .tooltip(
                                                e.record.relative.to_string_lossy().to_string(),
                                            )
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                this.ui.trash_open = false;
                                                this.review_draft(i, w, cx);
                                            })),
                                    )
                            })),
                    )
                },
            );
        div()
            .absolute()
            .inset_0()
            .bg(rgba(0x00000066))
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(top)
            .child(content)
            .into_any_element()
    }
    fn picker_search(&self, command: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(px(50.))
            .px_3()
            .border_b_1()
            .border_color(self.border())
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
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(rgba(0x88888822))
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
