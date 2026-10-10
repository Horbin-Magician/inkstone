//! Settings navigation, common layout controls, search and editor preferences view.
use super::focus_reveal::FocusReveal;
use super::ui::icon;
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Selectable, button::*, slider::Slider};

pub(super) fn setting_switch(id: impl Into<ElementId>) -> gpui_component::switch::Switch {
    use gpui_component::Sizable;
    gpui_component::switch::Switch::new(id).large()
}

#[derive(Clone, Copy)]
enum EditorSetting {
    ReadableWidth,
    StrictBreaks,
    FoldHeadings,
    FoldIndentation,
    LineNumbers,
    IndentGuides,
    PairBrackets,
    PairMarkdown,
    SmartLists,
    UseTabs,
}

impl Workspace {
    pub(super) fn settings_label(
        &self,
        id: &'static str,
        name: &str,
        description: &str,
    ) -> Stateful<Div> {
        let description = description.to_string();
        div()
            .id((ElementId::from(id), "name"))
            .role(gpui::Role::Label)
            .aria_label(name.to_string())
            .text_size(px(MIN_UI_FONT_SIZE))
            .line_height(relative(1.3))
            .child(name.to_string())
            .when(!description.is_empty(), |label| {
                label
                    .aria_description(description.clone())
                    .tooltip(move |window, cx| {
                        let description = description.clone();
                        gpui_component::tooltip::Tooltip::element(move |_, _| {
                            div()
                                .max_w(px(360.))
                                .whitespace_normal()
                                .child(description.clone())
                        })
                        .build(window, cx)
                    })
            })
    }

    pub(super) fn settings_content(&self) -> Stateful<Div> {
        div()
            .id("settings-content")
            .track_scroll(&self.ui.settings_scroll)
            .relative()
            // This child moves with the scrolled content. Anchor the painted
            // track to the handle's viewport, not to the child's layout bounds.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(gpui_base::Scrollbar::vertical(&self.ui.settings_scroll)),
            )
            .overflow_y_scroll()
            .min_h_0()
            .h_full()
            .flex_1()
            .min_w_0()
            .px(px(28.))
            .py(px(28.))
            .flex()
            .flex_col()
    }

    pub(super) fn settings_row(
        &self,
        id: &'static str,
        name: &str,
        description: &str,
        control: impl IntoElement,
        divider: bool,
        padding: f32,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_4()
            .flex_shrink_0()
            .py(px(padding))
            .relative()
            .when(divider, |row| {
                row.child(
                    div()
                        .absolute()
                        .top(px(-1.))
                        .left_0()
                        .right_0()
                        .h(px(1.))
                        .bg(self.border()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.settings_label(id, name, description)),
            )
            .child(FocusReveal::new(
                (ElementId::from(id), "focus"),
                &self.ui.settings_scroll,
                control,
            ))
            .into_any_element()
    }

    pub(super) fn settings_group(&self, title: &str, rows: Vec<AnyElement>) -> AnyElement {
        div()
            .w_full()
            .flex_shrink_0()
            .max_w(px(700.))
            .mx_auto()
            .mt_4()
            .child(
                div()
                    .px_4()
                    .mb_3()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .px_5()
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .bg(crate::theme::palette(self.ui.prefs.light).surface)
                    .children(rows),
            )
            .into_any_element()
    }

    fn editor_setting_row(
        &self,
        setting: EditorSetting,
        divider: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use EditorSetting::*;
        let p = &self.ui.prefs;
        let (id, name, description, checked) = match setting {
            ReadableWidth => (
                "width-setting",
                "限制行宽",
                "限制正文行宽，让较长的段落更易阅读。",
                p.readable_width,
            ),
            StrictBreaks => (
                "strict-line-breaks",
                "严格换行",
                "按 Markdown 标准处理换行，阅读视图中的单个换行符不另起一行。",
                p.strict_line_breaks,
            ),
            FoldHeadings => (
                "fold-headings-setting",
                "折叠标题",
                "允许将章节正文折叠到标题行。",
                p.fold_headings,
            ),
            FoldIndentation => (
                "fold-indentation-setting",
                "折叠缩进",
                "允许折叠列表等缩进内容。",
                p.fold_indentation,
            ),
            LineNumbers => ("line-number-setting", "行号", "", p.line_numbers),
            IndentGuides => (
                "indent-guides-setting",
                "缩进参考线",
                "",
                p.show_indent_guides,
            ),
            PairBrackets => (
                "pair-brackets",
                "自动补全英文标点符号",
                "输入英文括号或引号时补齐另一侧。",
                p.auto_pair_brackets,
            ),
            PairMarkdown => (
                "pair-markdown",
                "自动补全 Markdown 语法",
                "输入强调和代码标记时自动配对。",
                p.auto_pair_markdown,
            ),
            SmartLists => (
                "smart-lists-setting",
                "智能列表",
                "换行时延续列表，并调整有序列表编号。",
                p.smart_lists,
            ),
            UseTabs => (
                "use-tabs-setting",
                "使用制表符",
                "开启后使用制表符缩进，关闭后使用空格。",
                p.use_tabs,
            ),
        };
        let control = setting_switch(id)
            .accessibility_label(name)
            .checked(checked)
            .on_click(cx.listener(move |this, enabled: &bool, window, cx| {
                let p = &mut this.ui.prefs;
                match setting {
                    ReadableWidth => p.readable_width = *enabled,
                    StrictBreaks => p.strict_line_breaks = *enabled,
                    FoldHeadings => p.fold_headings = *enabled,
                    FoldIndentation => p.fold_indentation = *enabled,
                    LineNumbers => p.line_numbers = *enabled,
                    IndentGuides => p.show_indent_guides = *enabled,
                    PairBrackets => p.auto_pair_brackets = *enabled,
                    PairMarkdown => p.auto_pair_markdown = *enabled,
                    SmartLists => p.smart_lists = *enabled,
                    UseTabs => p.use_tabs = *enabled,
                }
                this.apply_editor_preferences(window, cx);
            }));
        self.settings_row(id, name, description, control, divider, 20.)
    }
}

impl Workspace {
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
            .child(Input::new(&self.ui.settings_filter).w_full())
            .children(
                [
                    (5, "外观", "palette"),
                    (6, "界面", "monitor"),
                    (0, "编辑器", "pencil"),
                    (2, "文件与链接", "folder"),
                    (7, "备份与恢复", "history"),
                    (8, "云同步", "folder"),
                    (1, "快捷键", "command"),
                    (9, "关于与更新", "info"),
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
                            this.ui
                                .settings_filter
                                .update(cx, |input, cx| input.set_value("", w, cx));
                            this.ui.settings_tab = i;
                            this.ui.settings_scroll.set_offset(Point::default());
                            if i == 2 {
                                this.prepare_file_settings(w, cx);
                            }
                            this.ui.hotkey_recording = None;
                            this.ui.hotkey_recording_focus = None;
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }
    pub(super) fn settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self
            .ui
            .settings_filter
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        if !query.is_empty() {
            let entries = [
                (5, "外观 · 基础颜色", "主题 深色 浅色 跟随系统 配色 theme"),
                (5, "外观 · 字体", "界面字体 正文字体 等宽字体 代码 font"),
                (5, "外观 · 字体大小", "字号 缩放 快速调整 Ctrl 滚轮 zoom"),
                (6, "界面 · 显示标签页标题栏", "文件标题 导航控件 header"),
                (
                    0,
                    "编辑器 · 默认视图与编辑模式",
                    "编辑 阅读 实时预览 源码 Markdown",
                ),
                (
                    0,
                    "编辑器 · 显示",
                    "可读行宽 行间距 行距 倍数 line spacing 严格换行 折叠标题 折叠缩进 行号 缩进参考线",
                ),
                (
                    0,
                    "编辑器 · 自动补全",
                    "英文标点符号 括号 引号 Markdown 语法 强调 代码 自动配对",
                ),
                (
                    0,
                    "编辑器 · 智能列表与缩进",
                    "换行 编号 制表符宽度 空格 tab",
                ),
                (
                    9,
                    "关于与更新 · 检查更新",
                    "版本 下载 安装 更新说明 update version",
                ),
                (2, "文件与链接 · 存放位置", "新笔记 附件 目录 路径 文件夹"),
                (
                    2,
                    "文件与链接 · 链接格式",
                    "双链 Markdown 最短路径 相对路径 绝对路径 自动更新",
                ),
                (
                    7,
                    "备份与恢复 · 整库备份",
                    "存放位置 手动 每天 每周 自动 校验 SHA-256 backup",
                ),
                (
                    8,
                    "云同步 · WebDAV",
                    "服务器 账号 密码 连接 同步 cloud sync",
                ),
                (7, "备份与恢复 · 恢复笔记库", "还原 新目录 数据恢复 restore"),
                (
                    7,
                    "备份与恢复 · 版本历史保留",
                    "天数 时间 容量 限制 清理 历史 retention",
                ),
                (
                    1,
                    "快捷键 · 命令绑定",
                    "键盘 组合键 冲突 修改 重置 恢复默认 hotkey shortcut",
                ),
            ];
            let rows: Vec<_> = entries
                .into_iter()
                .enumerate()
                .filter(|(_, (_, label, terms))| {
                    let haystack = format!("{label} {terms}").to_lowercase();
                    query.split_whitespace().all(|word| haystack.contains(word))
                })
                .map(|(id, (tab, label, _))| {
                    Button::new(("settings-search-result", id))
                        .ghost()
                        .w_full()
                        .h_auto()
                        .accessibility_label(label)
                        .child(div().w_full().py_2().flex().flex_col().gap_1().child(label))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.ui
                                .settings_filter
                                .update(cx, |input, cx| input.set_value("", w, cx));
                            this.ui.settings_tab = tab;
                            this.ui.settings_scroll.set_offset(Point::default());
                            this.ui.hotkey_recording = None;
                            this.ui.hotkey_recording_focus = None;
                            if tab == 2 {
                                this.prepare_file_settings(w, cx);
                            }
                            cx.notify();
                        }))
                })
                .collect();
            let empty = rows.is_empty();
            return div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(self.settings_nav(cx))
                .child(
                    self.settings_content()
                        .gap_2()
                        .child("搜索结果")
                        .when(empty, |s| s.child("未找到相关设置"))
                        .children(rows),
                )
                .into_any_element();
        }
        if self.ui.settings_tab == 9 {
            return self.update_settings_panel(cx);
        }
        if self.ui.settings_tab == 8 {
            return self.cloud_sync_settings_panel(cx);
        }
        if self.ui.settings_tab == 7 {
            return self.backup_settings_panel(cx);
        }
        if matches!(self.ui.settings_tab, 5 | 6) {
            return self.appearance_settings_panel(self.ui.settings_tab == 6, cx);
        }
        if self.ui.settings_tab == 2 {
            return self.file_settings_panel(cx);
        }
        if self.ui.settings_tab == 1 {
            return self.hotkey_settings_panel(cx);
        }
        let reading = self.ui.prefs.default_reading;
        let weak = cx.entity().downgrade();
        let default_view = Button::new("default-view-mode")
            .accessibility_label("新标签页的默认视图")
            .h(px(32.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if reading {
                        "阅读视图"
                    } else {
                        "编辑视图"
                    })
                    .child(icon("chevrons-up-down").size(px(14.))),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in [(false, "编辑视图"), (true, "阅读视图")] {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(reading == value)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    weak.update(cx, |this, cx| this.set_default_view(value, cx));
                            }),
                    );
                }
                menu
            });
        let live = self.ui.prefs.default_live_preview;
        let weak = cx.entity().downgrade();
        let editing_mode = Button::new("default-editing-mode")
            .accessibility_label("默认编辑模式")
            .h(px(32.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if live { "实时预览" } else { "源码模式" })
                    .child(icon("chevrons-up-down").size(px(14.))),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in [(true, "实时预览"), (false, "源码模式")] {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(live == value).on_click(
                        move |_, _, cx| {
                            let _ = weak
                                .update(cx, |this, cx| this.set_default_editing_mode(value, cx));
                        },
                    ));
                }
                menu
            });
        let spacing = self.ui.prefs.line_spacing;
        let weak = cx.entity().downgrade();
        let line_spacing = Button::new("line-spacing-setting")
            .accessibility_label("行间距")
            .h(px(32.))
            .label(format!("{spacing} 倍"))
            .dropdown_menu(move |mut menu, _, _| {
                for value in [1., 1.25, 1.5, 1.75, 2., 2.5, 3.] {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{value} 倍"))
                            .checked(spacing == value)
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.ui.prefs.line_spacing = value;
                                    this.apply_editor_preferences(window, cx);
                                });
                            }),
                    );
                }
                menu
            });
        let mut display: Vec<_> = [
            EditorSetting::ReadableWidth,
            EditorSetting::StrictBreaks,
            EditorSetting::FoldHeadings,
            EditorSetting::FoldIndentation,
            EditorSetting::LineNumbers,
            EditorSetting::IndentGuides,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, setting)| self.editor_setting_row(setting, i > 0, cx))
        .collect();
        display.push(self.settings_row(
            "line-spacing-row",
            "行间距",
            "调整编辑与阅读视图的正文行距，默认为 1.5 倍。",
            line_spacing,
            true,
            20.,
        ));
        let mut behavior: Vec<_> = [
            EditorSetting::PairBrackets,
            EditorSetting::PairMarkdown,
            EditorSetting::SmartLists,
            EditorSetting::UseTabs,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, setting)| self.editor_setting_row(setting, i > 0, cx))
        .collect();
        behavior.push(
            self.settings_row(
                "tab-size-row",
                &format!("制表符宽度  {}", self.ui.prefs.tab_size),
                "设置制表符对应的空格数。",
                div()
                    .w(px(160.))
                    .debug_selector(|| "tab-width-slider".into())
                    .child(Slider::new(&self.ui.tab_width).accessibility_label("制表符宽度")),
                true,
                20.,
            ),
        );
        let card = crate::theme::palette(self.ui.prefs.light).surface;
        div()
            .flex()
            .gap_0()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                self.settings_content()
                    .gap_2()
                    .child(
                        div()
                            .px_5()
                            .flex_shrink_0()
                            .rounded(px(12.))
                            .bg(card)
                            .child(self.settings_row(
                                "default-view-row",
                                "新标签页的默认视图",
                                "",
                                default_view,
                                false,
                                20.,
                            ))
                            .child(self.settings_row(
                                "default-mode-row",
                                "默认编辑模式",
                                "",
                                editing_mode,
                                true,
                                20.,
                            )),
                    )
                    .child(self.settings_group("显示", display))
                    .child(self.settings_group("行为", behavior))
                    .child(FocusReveal::new(
                        "settings-recovery-focus",
                        &self.ui.settings_scroll,
                        Button::new("settings-recovery")
                            .ghost()
                            .label("管理文件恢复")
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.ui.settings = false;
                                this.execute_command(16, w, cx);
                            })),
                    )),
            )
            .into_any_element()
    }
}
