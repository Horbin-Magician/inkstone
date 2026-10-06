//! Stable command IDs are persisted in preferences; they are not array positions.

use super::ui::NameMode;
use super::*;

pub(super) const COMMANDS: &[(usize, &str, &str)] = &[
    (0, "新建笔记", "Ctrl+N"),
    (1, "打开另一个笔记库", "Ctrl+Shift+O"),
    (2, "快速切换", "Ctrl+O"),
    (3, "搜索所有文件", "Ctrl+Shift+F"),
    (4, "保存当前更改", "Ctrl+S"),
    (5, "关闭当前标签页", "Ctrl+W"),
    (6, "切换阅读视图", "Ctrl+E"),
    (7, "切换源码模式", ""),
    (8, "重命名文件 / 移动文件", "F2"),
    (9, "创建文件夹", ""),
    (10, "删除当前文件", ""),
    (11, "另存为副本", ""),
    (12, "切换左侧栏", ""),
    (13, "切换右侧栏", "Ctrl+Shift+R"),
    (14, "打开设置", "Ctrl+,"),
    (15, "为当前文件添加 / 移除书签", ""),
    (16, "文件恢复（草稿、历史、回收站、同步备份）", ""),
    (17, "重新打开已关闭标签页", "Ctrl+Shift+T"),
    (18, "在文件管理器中显示", ""),
    (19, "复制文件路径", ""),
    (20, "返回", "Alt+Left"),
    (21, "前进", "Alt+Right"),
    (22, "切换浅色 / 深色主题", ""),
    (23, "查找并替换", "Ctrl+H"),
    (24, "切换粗体", "Ctrl+B"),
    (25, "切换斜体", "Ctrl+I"),
    (26, "插入内部链接", ""),
    (27, "固定 / 取消固定标签页", ""),
    (30, "插入附件", ""),
    (31, "向右分屏", "Ctrl+\\"),
    (32, "向下分屏", ""),
    (33, "关闭分屏", ""),
    (34, "新建标签页", "Ctrl+T"),
    (35, "切换任务状态", "Ctrl+L"),
    (36, "折叠 / 展开当前标题或列表", ""),
    (37, "折叠所有标题和列表", ""),
    (38, "展开所有标题和列表", ""),
    (39, "打开命令面板", "Ctrl+P"),
    (40, "下一个标签页", "Ctrl+Tab"),
    (41, "上一个标签页", "Ctrl+Shift+Tab"),
    (42, "复制当前笔记", ""),
    (45, "切换删除线", ""),
    (46, "切换高亮", ""),
    (47, "切换行内代码", ""),
    (48, "切换引用", ""),
    (49, "切换无序列表", ""),
    (50, "切换有序列表", ""),
    (51, "插入代码块", ""),
    (52, "设为正文", ""),
    (53, "设为一级标题", ""),
    (54, "设为二级标题", ""),
    (55, "设为三级标题", ""),
    (56, "设为四级标题", ""),
    (57, "设为五级标题", ""),
    (58, "设为六级标题", ""),
    (59, "插入 Markdown 链接", "Ctrl+K"),
    (60, "插入表格", ""),
    (61, "表格：在下方插入行", ""),
    (62, "表格：删除当前行", ""),
    (63, "表格：在右侧插入列", ""),
    (64, "表格：删除当前列", ""),
    (65, "表格：当前列左对齐", ""),
    (66, "表格：当前列居中", ""),
    (67, "表格：当前列右对齐", ""),
    (68, "关闭其他标签页", ""),
    (69, "关闭右侧标签页", ""),
    (70, "关闭所有未固定标签页", ""),
    (71, "插入 Callout 提示块", ""),
    (72, "插入脚注", ""),
    (73, "编辑光标处的脚注", ""),
    (74, "在文件列表中显示当前文件", ""),
    (79, "选择下一个相同文本", "Ctrl+D"),
    (80, "选择所有相同文本", "Ctrl+Shift+L"),
    (81, "上移当前行", "Alt+Up"),
    (82, "下移当前行", "Alt+Down"),
    (83, "向上复制当前行", "Alt+Shift+Up"),
    (84, "向下复制当前行", "Alt+Shift+Down"),
    (85, "在新标签页中打开当前笔记", ""),
    (86, "切换到第 1 个标签页", "Ctrl+1"),
    (87, "切换到第 2 个标签页", "Ctrl+2"),
    (88, "切换到第 3 个标签页", "Ctrl+3"),
    (89, "切换到第 4 个标签页", "Ctrl+4"),
    (90, "切换到第 5 个标签页", "Ctrl+5"),
    (91, "切换到第 6 个标签页", "Ctrl+6"),
    (92, "切换到第 7 个标签页", "Ctrl+7"),
    (93, "切换到第 8 个标签页", "Ctrl+8"),
    (94, "切换到最后一个标签页", "Ctrl+9"),
    (95, "删除当前行", "Ctrl+Shift+K"),
    (96, "选中当前行", "Alt+L"),
    (98, "切换注释", "Ctrl+/"),
    (99, "查看当前笔记版本历史", ""),
    (100, "比较并处理外部修改", ""),
    (101, "备份当前笔记库", ""),
    (102, "从备份恢复为新笔记库", ""),
    (103, "导出 Markdown 与关联笔记、附件", ""),
    (104, "导出 HTML（浏览器打印 / PDF）", ""),
    (105, "附件管理：引用、预览与清理", ""),
    (106, "检查本地链接健康", ""),
    (107, "全库文本替换（先预览）", ""),
    (108, "批量重命名或合并标签（先预览）", ""),
    (109, "切换专注模式", ""),
    (110, "快速记录（时间命名的新笔记）", ""),
    (111, "可视化编辑光标所在表格", ""),
];

pub(super) fn command(id: usize) -> Option<&'static (usize, &'static str, &'static str)> {
    COMMANDS.iter().find(|entry| entry.0 == id)
}

impl Workspace {
    pub(super) fn filtered_commands(&self, cx: &App) -> Vec<(usize, &'static str, &'static str)> {
        let query = self.ui.command.read(cx).value().to_lowercase();
        COMMANDS
            .iter()
            .copied()
            .filter(|(_, text, shortcut)| {
                text.to_lowercase().contains(&query) || shortcut.to_lowercase().contains(&query)
            })
            .collect()
    }
    pub(super) fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_open = true;
        self.ui.selected = 0;
        self.ui.command.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }
    pub(super) fn cycle_tab(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            return;
        }
        let n = self.tabs.len();
        let i = self.active.unwrap_or(0);
        self.activate_tab((i + if backwards { n - 1 } else { 1 }) % n, window, cx);
    }
    fn wrap_selection(
        &mut self,
        left: &str,
        right: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
        {
            return;
        }
        if !self.ensure_active_note(window, cx) {
            return;
        }
        let Some(pane) = self.current_pane() else {
            return;
        };
        pane.update(cx, |pane, cx| {
            pane.reading = false;
            pane.editor.update(cx, |state, cx| {
                let range = state.selected_range();
                let text = state.value();
                if let Some(edit) =
                    inkstone_core::markdown_edit::inline_format(&text, range, left, right)
                {
                    state.apply_source_edit(
                        edit.range,
                        &edit.replacement,
                        edit.selection,
                        window,
                        cx,
                    );
                }
                state.focus(window, cx);
            });
            cx.notify();
        });
    }
    fn format_block(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
            || !self.ensure_active_note(window, cx)
        {
            return;
        }
        use inkstone_core::markdown_edit::BlockFormat;
        let format = match id {
            48 => BlockFormat::Quote,
            49 => BlockFormat::Bullet,
            50 => BlockFormat::Numbered,
            51 => BlockFormat::Code,
            52 => BlockFormat::Heading(0),
            71 => BlockFormat::Callout,
            _ => BlockFormat::Heading((id - 52) as u8),
        };
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| {
                p.reading = false;
                p.editor.update(cx, |s, cx| {
                    if let Some(edit) = inkstone_core::markdown_edit::block_format(
                        &s.value(),
                        s.selected_range(),
                        format,
                    ) {
                        s.apply_source_edit(
                            edit.range,
                            &edit.replacement,
                            edit.selection,
                            window,
                            cx,
                        );
                        s.focus(window, cx);
                    }
                });
                cx.notify();
            });
        }
    }
    fn insert_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
            || !self.ensure_active_note(window, cx)
        {
            return;
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| {
                let mut inserted = false;
                p.editor.update(cx, |s, cx| {
                    if let Some(edit) = inkstone_core::markdown_edit::insert_footnote(
                        &s.value(),
                        s.selected_range(),
                    ) && s.apply_source_edit(
                        edit.range,
                        &edit.replacement,
                        edit.selection,
                        window,
                        cx,
                    ) {
                        p.reading = false;
                        inserted = true;
                        s.focus(window, cx);
                    }
                });
                if inserted {
                    p.open_new_footnote(window, cx);
                }
                cx.notify();
            });
        }
    }
    fn insert_link(&mut self, wiki: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
            || !self.ensure_active_note(window, cx)
        {
            return;
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| {
                p.reading = false;
                p.editor.update(cx, |s, cx| {
                    if let Some(edit) = inkstone_core::markdown_edit::insert_link(
                        &s.value(),
                        s.selected_range(),
                        wiki,
                    ) {
                        let applied = s.apply_source_edit(
                            edit.range,
                            &edit.replacement,
                            edit.selection,
                            window,
                            cx,
                        );
                        if applied && wiki {
                            s.request_completions(window, cx);
                        }
                        s.focus(window, cx);
                    }
                });
                cx.notify();
            });
        }
    }
    fn edit_table(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
        {
            return;
        }
        if id == 60 && !self.ensure_active_note(window, cx) {
            return;
        }
        use inkstone_core::tables::Operation;
        let operation = match id {
            60 => Operation::Insert,
            61 => Operation::AddRow,
            62 => Operation::RemoveRow,
            63 => Operation::AddColumn,
            64 => Operation::RemoveColumn,
            65 => Operation::Left,
            66 => Operation::Center,
            _ => Operation::Right,
        };
        let mut applied = false;
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| {
                p.editor.update(cx, |s, cx| {
                    if let Some(edit) =
                        inkstone_core::tables::edit(&s.value(), s.selected_range(), operation)
                    {
                        applied = s.apply_source_edit(
                            edit.range,
                            &edit.replacement,
                            edit.selection,
                            window,
                            cx,
                        );
                        s.focus(window, cx);
                    }
                });
                if applied {
                    p.reading = false;
                }
                cx.notify();
            });
        }
        if !applied {
            self.notifications.publish(
                "请在独立表格的单元格中操作，并完成输入法组词；结构编辑暂支持最多 10 万个单元格。"
                    .into(),
            );
        }
    }
    pub(super) fn execute_command(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_open = false;
        match id {
            0 => self.focus_new(window, cx),
            1 => self.choose_vault(window, cx),
            2 => self.focus_search(false, window, cx),
            3 => self.focus_search(true, window, cx),
            4 => self.save_all(window, cx),
            5 => self.close_tab(window, cx),
            6 | 7 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        if id == 6 {
                            pane.reading = !pane.reading;
                        } else {
                            pane.reading = false;
                            pane.live = !pane.live;
                        }
                        pane.focus_view(window, cx);
                        cx.notify();
                    });
                }
            }
            8 => self.prompt_name(NameMode::Rename, window, cx),
            9 => self.prompt_name(NameMode::Folder, window, cx),
            10 => self.manage_note(true, window, cx),
            11 => self.save_copy(window, cx),
            12 => {
                self.ui.prefs.left_open = self.ui.focus_mode || !self.ui.prefs.left_open;
                self.ui.focus_mode = false;
            }
            13 => {
                self.ui.prefs.right_open = self.ui.focus_mode || !self.ui.prefs.right_open;
                self.ui.focus_mode = false;
            }
            109 => self.ui.focus_mode = !self.ui.focus_mode,
            110 => self.quick_capture(window, cx),
            111 => self.open_table_editor(window, cx),
            14 => {
                self.prepare_file_settings(window, cx);
                self.ui.settings = true;
                window.focus(&self.ui.modal_focus, cx);
            }
            15 => {
                if let Some(path) = self
                    .active
                    .and_then(|i| self.tabs.get(i))
                    .map(|t| t.path.clone())
                {
                    if self.ui.prefs.bookmarks.contains(&path) {
                        self.ui.prefs.bookmarks.retain(|p| p != &path);
                    } else {
                        self.ui.prefs.bookmarks.push(path);
                    }
                }
            }
            16 => self.show_recovery_hub(window, cx),
            99 => self.open_history(window, cx),
            100 => self.open_conflict_review(window, cx),
            101 => self.request_backup(window, cx),
            102 => self.choose_backup_restore(window, cx),
            105 => self.open_attachment_manager(window, cx),
            106 => self.open_link_health(window, cx),
            107 => self.open_bulk_edit(false, window, cx),
            108 => self.open_bulk_edit(true, window, cx),
            103 => self.choose_export(inkstone_core::vault::export::Format::Markdown, window, cx),
            104 => self.choose_export(inkstone_core::vault::export::Format::Html, window, cx),
            17 => {
                if let Some(closed) = self.ui.closed.pop() {
                    self.open_note_with_view(closed.view.path.clone(), Some(closed), window, cx);
                }
            }
            18 => {
                if let Some(v) = &self.vault {
                    let path = self
                        .active
                        .and_then(|i| self.tabs.get(i))
                        .map(|t| v.root.join(&t.path))
                        .unwrap_or(v.root.clone());
                    cx.reveal_path(&path);
                }
            }
            19 => {
                if let Some(t) = self.active.and_then(|i| self.tabs.get(i)) {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        t.path.to_string_lossy().to_string(),
                    ));
                }
            }
            20 => self.navigate(false, window, cx),
            21 => self.navigate(true, window, cx),
            22 => {
                let theme = if self.ui.prefs.light {
                    inkstone_core::preferences::ThemeMode::Dark
                } else {
                    inkstone_core::preferences::ThemeMode::Light
                };
                self.set_theme(theme, window, cx);
            }
            23 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        pane.reading = false;
                        pane.editor.update(cx, |s, cx| {
                            s.open_search(true, cx);
                            s.focus(window, cx);
                        });
                        cx.notify();
                    });
                }
            }
            24 => self.wrap_selection("**", "**", window, cx),
            25 => self.wrap_selection("*", "*", window, cx),
            26 => self.insert_link(true, window, cx),
            27 => {
                if self.views.secondary_focused
                    && let Some(split) = &mut self.views.split
                {
                    split.pinned = !split.pinned;
                } else if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
                    tab.pinned = !tab.pinned;
                }
            }
            30 => self.choose_attachments(window, cx),
            31 => self.split_active(false, window, cx),
            32 => self.split_active(true, window, cx),
            33 => self.close_split(window, cx),
            34 => self.new_blank(window, cx),
            35 => {
                if self.ui.name_mode.is_none()
                    && !self.ui.quick_open
                    && !self.ui.settings
                    && !self.ui.property_open
                    && self.ensure_active_note(window, cx)
                    && let Some(pane) = self.current_pane()
                {
                    pane.update(cx, |p, cx| p.toggle_task_line(window, cx));
                }
            }
            36..=38 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |p, cx| {
                        p.fold_sections(
                            match id {
                                37 => Some(true),
                                38 => Some(false),
                                _ => None,
                            },
                            window,
                            cx,
                        )
                    });
                }
            }
            39 => self.open_commands(window, cx),
            40 => self.cycle_tab(false, window, cx),
            41 => self.cycle_tab(true, window, cx),
            42 => self.duplicate_current(window, cx),
            45 => self.wrap_selection("~~", "~~", window, cx),
            46 => self.wrap_selection("==", "==", window, cx),
            47 => self.wrap_selection("`", "`", window, cx),
            48..=58 => self.format_block(id, window, cx),
            59 => self.insert_link(false, window, cx),
            60..=67 => self.edit_table(id, window, cx),
            68..=70 => self.close_tab_group(
                self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id),
                id - 68,
                window,
                cx,
            ),
            71 => self.format_block(id, window, cx),
            72 => self.insert_footnote(window, cx),
            73 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |p, cx| {
                        p.open_footnote(window, cx);
                    });
                }
            }
            74 => {
                self.ui.prefs.left_open = true;
                self.ui.left_mode = 0;
                self.reveal_current_file(cx);
            }
            79..=80 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        if !pane.reading {
                            pane.editor.update(cx, |editor, cx| {
                                if id == 79 {
                                    editor.select_next_occurrence(
                                        &gpui_base::input::SelectNextOccurrence,
                                        window,
                                        cx,
                                    );
                                } else {
                                    editor.select_all_occurrences(
                                        &gpui_base::input::SelectAllOccurrences,
                                        window,
                                        cx,
                                    );
                                }
                                editor.focus(window, cx);
                            });
                        }
                    });
                }
            }
            81..=82 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| pane.move_lines(id == 82, window, cx));
                }
            }
            83..=84 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| pane.copy_lines(id == 84, window, cx));
                }
            }
            85 => self.open_current_note_in_new_tab(window, cx),
            98 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        if !pane.reading {
                            pane.editor.update(cx, |editor, cx| {
                                editor.apply_selection_transform(
                                    inkstone_core::comments::toggle,
                                    window,
                                    cx,
                                );
                                editor.focus(window, cx);
                            });
                        }
                    });
                }
            }
            95..=96 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        if !pane.reading {
                            pane.editor.update(cx, |editor, cx| {
                                if id == 95 {
                                    editor.delete_lines(window, cx);
                                } else {
                                    editor.select_lines(window, cx);
                                }
                                editor.focus(window, cx);
                            });
                        }
                    });
                }
            }
            86..=94 => {
                let index = if id == 94 {
                    self.tabs.len().checked_sub(1)
                } else {
                    (id - 86 < self.tabs.len()).then_some(id - 86)
                };
                if let Some(index) = index {
                    self.focus_primary(index, window, cx);
                }
            }
            _ => (),
        }
        self.persist_workspace(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn command_buttons_render_with_keyboard_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.open_commands(window, cx))
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1000.), px(600.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.command
                    .update(cx, |s, cx| s.set_value("专注", window, cx));
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[test]
    fn menu_labels_follow_stable_ids_across_removed_commands() {
        assert_eq!(command(31).unwrap().1, "向右分屏");
        assert_eq!(command(32).unwrap().1, "向下分屏");
        assert_eq!(command(42).unwrap().1, "复制当前笔记");
        assert_eq!(command(98).unwrap().1, "切换注释");
        assert!(command(28).is_none());
        assert!(command(usize::MAX).is_none());
        let ids: std::collections::HashSet<_> = COMMANDS.iter().map(|entry| entry.0).collect();
        assert_eq!(ids.len(), COMMANDS.len());
    }
}
