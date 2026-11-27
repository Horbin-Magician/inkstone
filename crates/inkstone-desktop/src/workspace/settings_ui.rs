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

/// Settings navigation sections: (group title, [(tab index, label, icon)]).
type SettingsNavItem = (usize, &'static str, &'static str);

const SETTINGS_SECTIONS: &[(&str, &[SettingsNavItem])] = &[
    ("个性化", &[(5, "外观", "palette"), (6, "界面", "monitor")]),
    (
        "编辑",
        &[
            (0, "编辑器", "pencil"),
            (2, "文件与链接", "folder"),
            (1, "快捷键", "command"),
        ],
    ),
    (
        "数据与服务",
        &[
            (7, "备份与恢复", "history"),
            (8, "云同步", "cloud"),
            (10, "Agent", "sparkles"),
        ],
    ),
    ("应用", &[(9, "关于与更新", "info")]),
];

pub(super) struct SearchTarget {
    id: &'static str,
    pending: std::rc::Rc<std::cell::Cell<bool>>,
    _timer: Task<()>,
}

// Each result points to a rendered setting. Aliases include familiar names and shortcuts.
const SEARCH_SETTINGS: &[(usize, &str, &str, &str)] = &[
    (
        5,
        "theme-row",
        "基础颜色",
        "主题 深色 浅色 跟随系统 配色 theme",
    ),
    (5, "interface-font-row", "界面字体", "font"),
    (5, "text-font-row", "正文字体", "font"),
    (5, "code-font-row", "等宽字体", "代码 font"),
    (5, "font-size-row", "字体大小", "字号 缩放 zoom"),
    (
        5,
        "quick-font-row",
        "快速调整字体大小",
        "字号 缩放 Ctrl 滚轮 zoom",
    ),
    (
        6,
        "view-header-row",
        "显示标签页标题栏",
        "文件标题 导航控件 header",
    ),
    (0, "default-view-row", "新标签页的默认视图", "阅读 编辑"),
    (
        0,
        "default-mode-row",
        "默认编辑模式",
        "实时预览 源码 Markdown",
    ),
    (0, "width-setting", "限制行宽", "可读行宽 显示"),
    (
        0,
        "line-spacing-row",
        "正文行距",
        "行间距 倍数 line spacing 显示",
    ),
    (0, "strict-line-breaks", "严格换行", "Markdown 显示"),
    (0, "fold-headings-setting", "折叠标题", "显示"),
    (0, "fold-indentation-setting", "折叠缩进", "显示"),
    (0, "line-number-setting", "行号", "显示"),
    (0, "indent-guides-setting", "缩进参考线", "显示"),
    (
        0,
        "pair-brackets",
        "自动补全英文标点符号",
        "括号 引号 自动配对",
    ),
    (
        0,
        "pair-markdown",
        "自动补全 Markdown 语法",
        "强调 代码 自动配对",
    ),
    (0, "smart-lists-setting", "智能列表", "换行 编号"),
    (0, "use-tabs-setting", "使用制表符", "缩进 空格 tab"),
    (0, "tab-size-row", "制表符宽度", "缩进 空格 tab"),
    (
        1,
        "hotkey-search",
        "快捷键命令绑定",
        "键盘 组合键 冲突 修改 重置 恢复默认 hotkey shortcut",
    ),
    (
        2,
        "note-location-label",
        "新建笔记的存放位置",
        "目录 路径 文件夹",
    ),
    (
        2,
        "attachment-location-label",
        "附件默认存放路径",
        "目录 存放位置 文件夹",
    ),
    (
        2,
        "link-format",
        "内部链接类型",
        "格式 最短路径 相对路径 绝对路径",
    ),
    (2, "use-wikilinks", "使用双链语法", "格式 Markdown"),
    (2, "update-links-label", "自动更新内部链接", "重命名 移动"),
    (
        7,
        "backup-directory-row",
        "备份存放位置",
        "整库备份 文件夹 backup",
    ),
    (
        7,
        "backup-now-row",
        "立即备份并校验",
        "整库备份 手动 SHA-256 backup",
    ),
    (
        7,
        "backup-schedule-label",
        "备份频率",
        "整库备份 每天 每周 自动 backup",
    ),
    (
        7,
        "backup-restore-row",
        "从备份恢复笔记库",
        "还原 新目录 数据恢复 restore",
    ),
    (
        7,
        "history-days-row",
        "版本历史保留时间",
        "天数 清理 retention",
    ),
    (
        7,
        "history-size-row",
        "版本历史容量上限",
        "限制 清理 retention",
    ),
    (
        8,
        "webdav-url-label",
        "服务器目录地址",
        "WebDAV URL cloud sync",
    ),
    (8, "webdav-username-label", "用户名", "WebDAV 账号"),
    (8, "webdav-password-label", "应用专用密码", "WebDAV 密码"),
    (8, "webdav-actions", "测试连接与保存配置", "WebDAV 服务器"),
    (8, "webdav-auto-label", "自动同步", "WebDAV"),
    (8, "webdav-poll-label", "远端检查间隔", "WebDAV 轮询"),
    (
        9,
        "update-actions",
        "检查更新",
        "版本 下载 安装 更新说明 update version",
    ),
    (
        10,
        "agent-current-config",
        "当前模型配置",
        "模型与权限 OpenAI GPT",
    ),
    (10, "agent-config-name", "配置名称", "模型"),
    (10, "agent-config-model", "模型名称", "OpenAI GPT"),
    (10, "agent-config-url", "模型服务地址", "URL OpenAI"),
    (10, "agent-config-key", "模型 API Key", "密钥 OpenAI"),
    (10, "agent-config-context", "上下文容量", "token"),
    (10, "agent-config-output", "输出上限", "token"),
    (10, "agent-config-excluded", "排除目录", "权限 范围"),
    (
        10,
        "agent-web-key",
        "DeepSeek 联网 API Key",
        "联网搜索 密钥",
    ),
    (10, "agent-actions", "测试连接与工具", "新增配置 保存配置"),
];

fn settings_tab_name(tab: usize) -> &'static str {
    SETTINGS_SECTIONS
        .iter()
        .flat_map(|(_, tabs)| tabs.iter())
        .find(|(index, _, _)| *index == tab)
        .unwrap()
        .1
}

fn settings_tab_icon(tab: usize) -> &'static str {
    SETTINGS_SECTIONS
        .iter()
        .flat_map(|(_, tabs)| tabs.iter())
        .find(|(i, _, _)| *i == tab)
        .map_or("settings", |(_, _, symbol)| symbol)
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
    pub(super) fn settings_target(&self, id: &'static str, element: Div) -> Div {
        let Some(target) = self.ui.settings_target.as_ref().filter(|t| t.id == id) else {
            return element;
        };
        let pending = target.pending.clone();
        let scroll = self.ui.settings_scroll.clone();
        element
            .bg(crate::theme::palette(self.ui.prefs.light).accent_tint)
            .rounded(px(4.))
            .debug_selector(|| "settings-highlight".into())
            .on_children_prepainted(move |children, window, _| {
                let Some(bounds) = children.first() else {
                    return;
                };
                let viewport = scroll.bounds();
                if viewport.size.height > px(0.) && pending.replace(false) {
                    let mut offset = scroll.offset();
                    offset.y += viewport.top() + viewport.size.height * 0.35 - bounds.top();
                    scroll.set_offset(offset);
                    window.refresh();
                }
            })
    }

    fn reveal_setting(
        &mut self,
        tab: usize,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_settings_tab(tab, window, cx);
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1600))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.ui.settings_target = None;
                cx.notify();
            });
        });
        self.ui.settings_target = Some(SearchTarget {
            id,
            pending: std::rc::Rc::new(std::cell::Cell::new(true)),
            _timer: timer,
        });
        window.focus(&self.ui.modal_focus, cx);
        cx.notify();
    }

    pub(super) fn settings_label(
        &self,
        id: &'static str,
        name: &str,
        description: &str,
    ) -> Stateful<Div> {
        let description = description.to_string();
        let muted = crate::theme::palette(self.ui.prefs.light).muted;
        self.settings_target(id, div())
            .id((ElementId::from(id), "name"))
            .role(gpui::Role::Label)
            .aria_label(name.to_string())
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .line_height(relative(1.35))
                    .child(name.to_string()),
            )
            .when(!description.is_empty(), |label| {
                label
                    .aria_description(description.clone())
                    .child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .line_height(relative(1.5))
                            .text_color(muted)
                            .child(description.clone()),
                    )
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
        let py = if description.is_empty() {
            padding
        } else {
            padding + 2.
        };
        div()
            .flex()
            .items_center()
            .gap_4()
            .flex_shrink_0()
            .py(px(py))
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
        let palette = crate::theme::palette(self.ui.prefs.light);
        div()
            .w_full()
            .flex_shrink_0()
            .max_w(px(700.))
            .mx_auto()
            .when(!title.is_empty(), |group| group.mt_5())
            .when(!title.is_empty(), |group| {
                group.child(
                    div()
                        .px_1()
                        .mb_2()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.muted)
                        .child(title.to_string()),
                )
            })
            .child(
                div()
                    .px_5()
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .children(rows),
            )
            .into_any_element()
    }

    /// Container for a row of mutually exclusive [`Self::segment`] buttons.
    pub(super) fn segmented(
        &self,
        id: impl Into<ElementId>,
        segments: impl IntoIterator<Item = impl IntoElement>,
    ) -> Stateful<Div> {
        let palette = crate::theme::palette(self.ui.prefs.light);
        div()
            .id(id)
            .flex()
            .flex_wrap()
            .gap(px(3.))
            .p(px(3.))
            .rounded(px(9.))
            .bg(palette.sidebar)
            .border_1()
            .border_color(palette.border)
            .children(segments)
    }

    pub(super) fn segment(
        &self,
        id: impl Into<ElementId>,
        label: &str,
        symbol: Option<&str>,
        active: bool,
    ) -> Button {
        let palette = crate::theme::palette(self.ui.prefs.light);
        Button::new(id)
            .ghost()
            .h(px(28.))
            .px_3()
            .rounded(px(6.))
            .selected(active)
            .accessibility_label(label.to_string())
            .when(active, |button| {
                button
                    .bg(palette.background)
                    .border_1()
                    .border_color(palette.border)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                    .text_color(if active {
                        palette.foreground
                    } else {
                        palette.muted
                    })
                    .children(symbol.map(|s| {
                        icon(s).size(px(15.)).text_color(if active {
                            palette.accent
                        } else {
                            palette.muted
                        })
                    }))
                    .child(label.to_string()),
            )
    }

    /// Page heading shown at the top of each settings page.
    pub(super) fn settings_title(&self, title: &str) -> AnyElement {
        div()
            .w_full()
            .max_w(px(700.))
            .mx_auto()
            .flex_shrink_0()
            .px_1()
            .mb_2()
            .text_size(px(22.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(title.to_string())
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
    pub(super) fn select_settings_tab(
        &mut self,
        tab: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui
            .settings_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.ui.settings_target = None;
        self.ui.settings_tab = tab;
        self.ui.settings_scroll.set_offset(Point::default());
        self.ui.hotkey_recording = None;
        self.ui.hotkey_recording_focus = None;
        if tab == 2 {
            self.prepare_file_settings(window, cx);
        }
        cx.notify();
    }

    pub(super) fn settings_nav(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = crate::theme::palette(self.ui.prefs.light);
        let accent = palette.accent;
        let muted = palette.muted;
        let groups = SETTINGS_SECTIONS;
        let current_tab = self.ui.settings_tab;
        let mut items: Vec<AnyElement> = Vec::new();
        for (section_title, tabs) in groups.iter() {
            if !section_title.is_empty() {
                items.push(
                    div()
                        .px_2p5()
                        .pt_3p5()
                        .pb_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(section_title.to_string())
                        .into_any_element(),
                );
            }
            for &(i, title, symbol) in tabs.iter() {
                let selected = current_tab == i;
                items.push(
                    Button::new(("settings-tab", i))
                        .ghost()
                        .h(px(36.))
                        .w_full()
                        .rounded(px(8.))
                        .selected(selected)
                        .toggled(selected)
                        .when(selected, |button| button.bg(palette.selected))
                        .accessibility_label(title)
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_2p5()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .when(selected, |d| {
                                    d.font_weight(FontWeight::MEDIUM).text_color(accent)
                                })
                                .child(
                                    div()
                                        .text_color(if selected { accent } else { muted })
                                        .child(ui::icon(symbol).size(px(16.))),
                                )
                                .child(title),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.select_settings_tab(i, w, cx);
                        }))
                        .into_any_element(),
                );
            }
        }
        div()
            .id("settings-navigation")
            .w(px(220.))
            .flex_shrink_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .px_3()
            .pt_3()
            .pb_4()
            .bg(self.side())
            .border_r_1()
            .border_color(self.border())
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .pb_2()
                    .child(Input::new(&self.ui.settings_filter).w_full()),
            )
            .children(items)
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
            let palette = crate::theme::palette(self.ui.prefs.light);
            let rows: Vec<_> = SEARCH_SETTINGS
                .iter()
                .copied()
                .filter(|(tab, _, name, terms)| {
                    let haystack =
                        format!("{} {name} {terms}", settings_tab_name(*tab)).to_lowercase();
                    query.split_whitespace().all(|word| haystack.contains(word))
                })
                .map(|(tab, target, item, _)| {
                    let section = settings_tab_name(tab);
                    let label = format!("{section} · {item}");
                    Button::new((ElementId::from("settings-search-result"), target))
                        .debug_selector(move || format!("settings-result-{target}"))
                        .ghost()
                        .w_full()
                        .h_auto()
                        .accessibility_label(label)
                        .child(
                            div()
                                .w_full()
                                .px_2()
                                .py_2()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(ui::icon(settings_tab_icon(tab)).text_color(palette.accent))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .child(item.to_string())
                                        .child(
                                            div()
                                                .text_color(palette.muted)
                                                .child(section.to_string()),
                                        ),
                                )
                                .child(icon("chevron-right").text_color(palette.muted)),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.reveal_setting(tab, target, w, cx);
                        }))
                })
                .collect();
            let count = rows.len();
            return div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(self.settings_nav(cx))
                .child(
                    self.settings_content().child(
                        div()
                            .w_full()
                            .max_w(px(700.))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.settings_title("搜索结果"))
                            .when(count == 0, |s| {
                                s.child(self.empty_state(
                                    "search",
                                    "未找到相关设置",
                                    "换个关键词试试，例如“字体”“同步”“快捷键”。",
                                ))
                            })
                            .when(count > 0, |s| {
                                s.child(
                                    div()
                                        .p_2()
                                        .flex()
                                        .flex_col()
                                        .rounded(px(12.))
                                        .bg(palette.surface)
                                        .border_1()
                                        .border_color(palette.border)
                                        .children(rows),
                                )
                            }),
                    ),
                )
                .into_any_element();
        }
        if self.ui.settings_tab == 10 {
            return self.agent_settings_panel(cx);
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
                    .child(icon("chevrons-up-down")),
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
                    .child(icon("chevrons-up-down")),
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
        div()
            .flex()
            .gap_0()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                self.settings_content()
                    .gap_2()
                    .child(self.settings_title("编辑器"))
                    .child(self.settings_group(
                        "",
                        vec![
                            self.settings_row(
                                "default-view-row",
                                "新标签页的默认视图",
                                "",
                                default_view,
                                false,
                                20.,
                            ),
                            self.settings_row(
                                "default-mode-row",
                                "默认编辑模式",
                                "",
                                editing_mode,
                                true,
                                20.,
                            ),
                        ],
                    ))
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

#[cfg(test)]
mod notification_tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn settings_search_reveals_exact_row_and_clears_highlight(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(980.), px(540.)));
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.settings = true;
                w.ui.settings_filter
                    .update(cx, |s, cx| s.set_value("行距", window, cx));
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let result = visual
            .debug_bounds("settings-result-line-spacing-row")
            .unwrap()
            .center();
        visual.simulate_click(result, Modifiers::default());
        for _ in 0..2 {
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        let target = visual.debug_bounds("settings-highlight").unwrap();
        handle
            .update(&mut visual, |w, _, cx| {
                assert!(w.ui.settings_filter.read(cx).value().is_empty());
                let viewport = w.ui.settings_scroll.bounds();
                assert!(w.ui.settings_scroll.offset().y < px(0.));
                assert!(
                    target.top() >= viewport.top() && target.bottom() <= viewport.bottom(),
                    "{target:?} {viewport:?}"
                );
            })
            .unwrap();
        visual.executor().advance_clock(Duration::from_secs(2));
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, _| {
                assert!(w.ui.settings_target.is_none())
            })
            .unwrap();
    }

    #[gpui::test]
    fn filters_notify_workspace_only_when_content_changes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let count = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = count.clone();
        let _subscription = handle
            .update(cx, |_, _, cx| {
                cx.observe(&cx.entity(), move |_, _, _| {
                    observed.set(observed.get() + 1)
                })
            })
            .unwrap();
        cx.run_until_parked();
        for settings in [true, false] {
            let before = count.get();
            handle
                .update(cx, |w, _, cx| {
                    let input = if settings {
                        &w.ui.settings_filter
                    } else {
                        &w.ui.hotkey_filter
                    };
                    input.update(cx, |_, cx| cx.notify());
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(
                count.get(),
                before,
                "cursor/geometry notifications must stay local"
            );
            handle
                .update(cx, |w, window, cx| {
                    let input = if settings {
                        &w.ui.settings_filter
                    } else {
                        &w.ui.hotkey_filter
                    };
                    input.update(cx, |s, cx| s.replace("保存", window, cx));
                })
                .unwrap();
            cx.run_until_parked();
            assert!(count.get() > before, "filter results must update");
        }
    }
}
