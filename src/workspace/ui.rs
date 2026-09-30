use super::*;
use gpui_component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::{
    Disableable, Icon, Selectable, TitleBar,
    button::*,
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    tree::Tree,
};
use inkstone::file_order::SortBy;
use inkstone::preferences::Preferences;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum NameMode {
    New,
    Rename,
    Folder,
    RenameFolder,
}

#[derive(PartialEq)]
pub(super) struct TabScrollKey {
    selected: Option<usize>,
    order: Vec<usize>,
    widths: [u32; 3],
}

#[derive(Clone, Debug)]
pub(super) struct ClosedTab {
    pub view: inkstone::preferences::ViewState,
    pub history: inkstone::preferences::Navigation,
}
impl ClosedTab {
    pub fn new(
        view: inkstone::preferences::ViewState,
        mut history: inkstone::preferences::Navigation,
    ) -> Self {
        if history.entries.is_empty() {
            history.visit(view.path.clone());
        }
        history.record_at(history.cursor, view.clone());
        Self { view, history }
    }
}
impl From<&str> for ClosedTab {
    fn from(path: &str) -> Self {
        Self::new(
            inkstone::preferences::ViewState {
                path: path.into(),
                ..Default::default()
            },
            Default::default(),
        )
    }
}
impl PartialEq<PathBuf> for ClosedTab {
    fn eq(&self, path: &PathBuf) -> bool {
        &self.view.path == path
    }
}

pub(super) struct UiState {
    pub prefs: Preferences,
    pub available_fonts: Arc<Vec<String>>,
    pub default_fonts: (SharedString, SharedString),
    pub font_selects: [Entity<SelectState<SearchableVec<super::appearance::FontChoice>>>; 3],
    _font_select_subscriptions: Vec<Subscription>,
    _appearance_subscription: Subscription,
    pub tab_width: Entity<SliderState>,
    pub font_size_slider: Entity<SliderState>,
    _font_size_subscription: Subscription,
    _tab_width_subscription: Subscription,
    pub closed: Vec<ClosedTab>,
    pub close_pending: std::collections::BTreeSet<usize>,
    pub window_close_requested: bool,
    pub pending_file_writes: usize,
    pub file_operation: bool,
    pub link_update: Option<LinkEdits>,
    pub link_update_scroll: UniformListScrollHandle,
    pub left_mode: usize,
    pub right_mode: usize,
    pub quick_open: bool,
    pub template_mode: bool,
    pub name_mode: Option<NameMode>,
    pub folder_target: Option<PathBuf>,
    pub settings: bool,
    pub settings_tab: usize,
    pub hotkey_recording: Option<usize>,
    pub hotkey_message: String,
    pub hotkey_filter: Entity<InputState>,
    _hotkey_subscription: Subscription,
    _hotkey_filter_subscription: Subscription,
    pub note_folder_input: Entity<InputState>,
    pub attachment_folder_input: Entity<InputState>,
    pub daily_inputs: [Entity<InputState>; 3],
    _daily_subscriptions: Vec<Subscription>,
    pub template_inputs: [Entity<InputState>; 3],
    _template_subscriptions: Vec<Subscription>,
    _location_subscriptions: Vec<Subscription>,
    pub property_open: bool,
    pub property_kind: inkstone::properties::Kind,
    pub property_error: String,
    _property_subscriptions: Vec<Subscription>,
    pub property_dates: [Entity<DatePickerState>; 2],
    _property_date_subscriptions: Vec<Subscription>,
    pub property_original: Option<String>,
    pub property_baseline: Option<(usize, String)>,
    pub property_key: Entity<InputState>,
    pub property_value: Entity<InputState>,
    pub property_list_entry: Entity<InputState>,
    pub tags_filter: Entity<InputState>,
    pub tags_focus: FocusHandle,
    pub tags_selected: Option<String>,
    pub tags_scroll: ScrollHandle,
    pub search_collapsed: std::collections::BTreeSet<PathBuf>,
    pub search_group_scroll: ScrollHandle,
    pub search_group_query: String,
    pub search_limit: usize,
    pub search_has_more: bool,
    pub search_loading: bool,
    pub search_error: String,
    pub search_signature: Option<(String, bool, SortBy, bool)>,
    _tags_filter_subscription: Subscription,
    _property_list_subscription: Subscription,
    pub more: bool,
    pub trash_open: bool,
    pub trash: Vec<inkstone::vault::TrashEntry>,
    pub command: Entity<InputState>,
    pub selected: usize,
    pub modal_scroll: ScrollHandle,
    pub settings_scroll: ScrollHandle,
    pub tab_scroll: ScrollHandle,
    pub tab_scroll_key: Option<TabScrollKey>,
    pub last_revealed_file: Option<(u64, PathBuf)>,
    pub middle_pressed_tab: Option<usize>,
    pub inline_title: Option<super::inline_title::InlineTitle>,
    pub modal_focus: FocusHandle,
    pub workspace_focus: FocusHandle,
    pub pending_command: Option<(PathBuf, usize)>,
    pub last_persisted: String,
    pub persisting: bool,
    pub folders: Vec<PathBuf>,
    pub tree_folders: Vec<PathBuf>,
    _command_subscription: Subscription,
}
impl UiState {
    pub(super) fn remember_closed(&mut self, closed: ClosedTab) {
        self.closed.push(closed);
        if self.closed.len() > 10 {
            self.closed.drain(..self.closed.len() - 10);
        }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let prefs = Preferences {
            light: Workspace::system_light(window),
            ..Default::default()
        };
        let mut available_fonts = cx.text_system().all_font_names();
        available_fonts.sort_by_key(|name| name.to_lowercase());
        available_fonts.dedup();
        let font_selects = std::array::from_fn(|_| {
            let choices: Vec<_> = std::iter::once(String::new())
                .chain(available_fonts.iter().cloned())
                .map(|name| super::appearance::FontChoice {
                    name,
                    missing: false,
                })
                .collect();
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(choices),
                    Some(gpui_component::IndexPath::new(0)),
                    window,
                    cx,
                )
                .searchable(true)
            })
        });
        let font_select_subscriptions = font_selects
            .iter()
            .enumerate()
            .map(|(role, select)| {
                cx.subscribe_in(
                    select,
                    window,
                    move |this,
                          _,
                          event: &SelectEvent<SearchableVec<super::appearance::FontChoice>>,
                          window,
                          cx| {
                        let SelectEvent::Confirm(Some(font)) = event else {
                            return;
                        };
                        match role {
                            0 => this.ui.prefs.interface_font = font.clone(),
                            1 => this.ui.prefs.text_font = font.clone(),
                            _ => this.ui.prefs.monospace_font = font.clone(),
                        }
                        this.apply_editor_preferences(window, cx);
                    },
                )
            })
            .collect();
        apply_theme(prefs.light, cx);
        let default_fonts = {
            let theme = gpui_component::Theme::global(cx);
            (theme.font_family.clone(), theme.mono_font_family.clone())
        };
        let weak = cx.entity().downgrade();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.update_system_theme(Workspace::system_light(window), window, cx);
            });
        });
        let font_size_slider = cx.new(|_| {
            SliderState::new()
                .min(10.)
                .max(30.)
                .step(1.)
                .default_value(16.)
        });
        let font_size_subscription = cx.subscribe_in(
            &font_size_slider,
            window,
            |this, _, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    this.ui.prefs.font_size = value.start().round().clamp(10., 30.);
                    this.apply_editor_preferences(window, cx);
                }
            },
        );
        let tab_width = cx.new(|_| {
            SliderState::new()
                .min(2.)
                .max(8.)
                .step(1.)
                .default_value(4.)
        });
        let tab_width_subscription = cx.subscribe_in(
            &tab_width,
            window,
            |this, _, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    this.ui.prefs.tab_size = (value.start().round() as usize).clamp(2, 8);
                    this.apply_editor_preferences(window, cx);
                }
            },
        );
        let workspace_focus = cx.focus_handle();
        window.focus(&workspace_focus, cx);
        let property_key = cx
            .new(|cx| InputState::new(window, cx).placeholder("属性名，如 tags、aliases、status"));
        let property_value = cx.new(|cx| {
            InputState::new(window, cx).placeholder("属性值；列表用逗号分隔，项目内含逗号时加引号")
        });
        let command = cx.new(|cx| InputState::new(window, cx).placeholder("选择命令…"));
        let tags_filter = cx.new(|cx| InputState::new(window, cx).placeholder("筛选标签…"));
        let tags_filter_subscription =
            cx.subscribe(&tags_filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ui.prefs.tags.query = input.read(cx).value().to_string();
                    this.ui.tags_selected = None;
                    this.ui.tags_scroll.set_offset(Point::default());
                    this.persist_workspace(cx);
                    cx.notify();
                }
            });
        let property_list_entry =
            cx.new(|cx| InputState::new(window, cx).placeholder("添加项目，按 Enter 确认"));
        let property_list_subscription = cx.subscribe_in(
            &property_list_entry,
            window,
            |this, _, event: &InputEvent, w, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && this.ui.property_open {
                    this.add_property_list_item(w, cx);
                }
            },
        );
        let property_dates = [false, true].map(|time| {
            cx.new(|cx| {
                let state = DatePickerState::new(window, cx);
                if time {
                    state
                        .time_precision(gpui_component::time_field::TimePrecision::Second)
                        .date_format("%Y-%m-%d %H:%M:%S")
                } else {
                    state.date_format("%Y-%m-%d")
                }
            })
        });
        let property_date_subscriptions = property_dates
            .iter()
            .enumerate()
            .map(|(i, picker)| {
                cx.subscribe_in(
                    picker,
                    window,
                    move |this, _, event: &DatePickerEvent, w, cx| {
                        let DatePickerEvent::Change(value) = event;
                        this.accept_property_date(i, *value, w, cx);
                    },
                )
            })
            .collect();
        let property_subscriptions = [&property_key, &property_value]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(
                    input,
                    window,
                    |this, _, event: &InputEvent, w, cx| match event {
                        InputEvent::Change => {
                            this.ui.property_error.clear();
                            this.sync_property_dates(w, cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } if this.ui.property_open => {
                            this.save_property(w, cx)
                        }
                        _ => (),
                    },
                )
            })
            .collect();
        let note_folder_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("笔记文件夹，如 收件箱"));
        let attachment_folder_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("附件文件夹，如 附件"));
        let location_subscriptions = [
            (&note_folder_input, false),
            (&attachment_folder_input, true),
        ]
        .into_iter()
        .map(|(input, attachment)| {
            cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    if attachment {
                        this.ui.prefs.locations.attachment_folder = value;
                    } else {
                        this.ui.prefs.locations.note_folder = value;
                    }
                    this.persist_workspace(cx);
                    cx.notify();
                }
            })
        })
        .collect();
        let hotkey_filter = cx.new(|cx| InputState::new(window, cx).placeholder("搜索快捷键命令…"));
        let daily_inputs = ["YYYY-MM-DD", "留空使用新建笔记位置", "可选，如 模板/日记"]
            .map(|placeholder| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)));
        let daily_subscriptions = daily_inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {
                cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        match i {
                            0 => this.ui.prefs.daily.format = value,
                            1 => this.ui.prefs.daily.folder = value,
                            _ => this.ui.prefs.daily.template = value,
                        }
                        this.persist_workspace(cx);
                        cx.notify();
                    }
                })
            })
            .collect();
        let template_inputs = ["模板文件夹", "YYYY-MM-DD", "HH:mm"]
            .map(|placeholder| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)));
        let template_subscriptions = template_inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {
                cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        match i {
                            0 => this.ui.prefs.templates.folder = value,
                            1 => this.ui.prefs.templates.date_format = value,
                            _ => this.ui.prefs.templates.time_format = value,
                        }
                        this.persist_workspace(cx);
                        cx.notify();
                    }
                })
            })
            .collect();
        let hotkey_filter_subscription = cx.observe(&hotkey_filter, |_, _, cx| cx.notify());
        let weak = cx.entity().downgrade();
        let window_id = window.window_handle().window_id();
        let hotkey_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() == window_id {
                let _ = weak.update(cx, |this, cx| this.handle_hotkey(event, window, cx));
            }
        });
        let subscription =
            cx.subscribe_in(&command, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.ui.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if let Some((id, _, _)) =
                        this.filtered_commands(cx).get(this.ui.selected).copied()
                    {
                        this.execute_command(id, window, cx);
                    }
                }
                _ => (),
            });
        Self {
            prefs,
            available_fonts: Arc::new(available_fonts),
            default_fonts,
            font_selects,
            _font_select_subscriptions: font_select_subscriptions,
            _appearance_subscription: appearance_subscription,
            tab_width,
            font_size_slider,
            _font_size_subscription: font_size_subscription,
            _tab_width_subscription: tab_width_subscription,
            closed: vec![],
            close_pending: Default::default(),
            window_close_requested: false,
            pending_file_writes: 0,
            file_operation: false,
            link_update: None,
            link_update_scroll: UniformListScrollHandle::new(),
            left_mode: 0,
            right_mode: 0,
            quick_open: false,
            template_mode: false,
            name_mode: None,
            folder_target: None,
            settings: false,
            settings_tab: 0,
            hotkey_recording: None,
            hotkey_message: String::new(),
            hotkey_filter,
            _hotkey_subscription: hotkey_subscription,
            _hotkey_filter_subscription: hotkey_filter_subscription,
            note_folder_input,
            attachment_folder_input,
            daily_inputs,
            _daily_subscriptions: daily_subscriptions,
            template_inputs,
            _template_subscriptions: template_subscriptions,
            _location_subscriptions: location_subscriptions,
            property_open: false,
            property_kind: Default::default(),
            property_error: String::new(),
            _property_subscriptions: property_subscriptions,
            property_dates,
            _property_date_subscriptions: property_date_subscriptions,
            property_original: None,
            property_baseline: None,
            property_key,
            property_value,
            property_list_entry,
            tags_filter,
            tags_focus: cx.focus_handle(),
            tags_selected: None,
            tags_scroll: ScrollHandle::new(),
            search_collapsed: Default::default(),
            search_group_scroll: ScrollHandle::new(),
            search_group_query: String::new(),
            search_limit: 200,
            search_has_more: false,
            search_loading: false,
            search_error: String::new(),
            search_signature: None,
            _tags_filter_subscription: tags_filter_subscription,
            _property_list_subscription: property_list_subscription,
            more: false,
            trash_open: false,
            trash: vec![],
            command,
            selected: 0,
            modal_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            tab_scroll: ScrollHandle::new(),
            tab_scroll_key: None,
            last_revealed_file: None,
            middle_pressed_tab: None,
            inline_title: None,
            modal_focus: cx.focus_handle(),
            workspace_focus,
            pending_command: None,
            last_persisted: String::new(),
            persisting: false,
            folders: vec![],
            tree_folders: vec![],
            _command_subscription: subscription,
        }
    }
}

pub(super) fn apply_theme(light: bool, cx: &mut App) {
    use gpui_component::{Theme, ThemeMode};
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        let bg = rgb(if light { 0xffffff } else { 0x262626 }).into();
        let side = rgb(if light { 0xf6f6f6 } else { 0x212121 }).into();
        let hover = rgb(if light { 0xe8e8e8 } else { 0x363636 }).into();
        let fg = rgb(if light { 0x0f0f0f } else { 0xd1d1d1 }).into();
        let muted = rgb(if light { 0x5c5c5c } else { 0x999999 }).into();
        let border = rgb(if light { 0xe6e6e6 } else { 0x363636 }).into();
        theme.font_size = px(14.);
        theme.radius = px(4.);
        theme.background = bg;
        theme.foreground = fg;
        theme.border = border;
        theme.input = side;
        theme.colors.list = side;
        theme.list_hover = hover;
        theme.list_active = hover;
        theme.list_active_border = hover;
        theme.muted = side;
        theme.muted_foreground = muted;
        theme.sidebar = side;
        theme.sidebar_foreground = fg;
        theme.title_bar = side;
        theme.title_bar_border = border;
        theme.popover = bg;
        theme.popover_foreground = fg;
        theme.secondary = hover;
        theme.secondary_hover = hover;
        theme.secondary_foreground = fg;
        theme.button = hover;
        theme.button_hover = hover;
        theme.button_foreground = fg;
        theme.accent = hover;
        theme.accent_foreground = fg;
        theme.list.active_highlight = true;
        theme.list_active = rgb(if light { 0xe8e8e8 } else { 0x2e2e2e }).into();
        theme.tokens.list_hover =
            Hsla::from(rgba(if light { 0x00000008 } else { 0xffffff08 })).into();
        theme.primary = rgb(0x3f9aca).into();
        theme.ring = rgb(0x3f9aca).into();
        theme.switch_thumb = rgb(0xffffff).into();
        theme.slider_thumb = rgb(0xffffff).into();
        theme.selection = rgba(0x7860b866).into();
        theme.scrollbar_thumb = rgb(if light { 0xcccccc } else { 0x484848 }).into();
    });
}
pub(super) fn setting_switch(id: impl Into<ElementId>) -> gpui_component::switch::Switch {
    use gpui_component::Sizable;
    gpui_component::switch::Switch::new(id).large()
}

#[derive(Clone, Copy)]
enum EditorSetting {
    InlineTitle,
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
    pub(super) fn settings_row(
        &self,
        name: &str,
        description: &str,
        control: impl IntoElement,
        divider: bool,
        padding: f32,
    ) -> AnyElement {
        div()
            .flex()
            .items_start()
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
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(16.9))
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(15.6))
                            .text_color(rgb(if self.ui.prefs.light {
                                0x5c5c5c
                            } else {
                                0x999999
                            }))
                            .child(description.to_string()),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
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
                    .bg(rgb(if self.ui.prefs.light {
                        0xfafafa
                    } else {
                        0x212121
                    }))
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
            InlineTitle => (
                "inline-title-setting",
                "页面内标题",
                "在正文上方显示可编辑的文件名。",
                p.show_inline_title,
            ),
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
            LineNumbers => (
                "line-number-setting",
                "行号",
                "在编辑区左侧显示行号。",
                p.line_numbers,
            ),
            IndentGuides => (
                "indent-guides-setting",
                "缩进参考线",
                "在缩进行之间显示参考线。",
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
                    InlineTitle => p.show_inline_title = *enabled,
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
                if matches!(setting, InlineTitle) {
                    this.persist_workspace(cx);
                    cx.notify();
                } else {
                    this.apply_editor_preferences(window, cx);
                }
            }));
        self.settings_row(name, description, control, divider, 20.)
    }
}

pub(super) fn icon(name: &str) -> Icon {
    let shape = match name {
        "square-pen" => Some(
            "M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7M16 3l5 5M9 15l1-5L18 2l4 4-8 8z",
        ),
        "monitor" => Some("M3 3h18v14H3zM12 17v4M8 21h8"),
        "command" => {
            Some("M9 7V5a2 2 0 1 0-2 2h10a2 2 0 1 0-2-2v14a2 2 0 1 0 2-2H7a2 2 0 1 0 2 2V7")
        }
        "chevrons-up-down" => Some("m8 9 4-4 4 4m-8 6 4 4 4-4"),
        "x" => Some("M6 6l12 12M18 6 6 18"),
        "file-plus" => Some(
            "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M12 11v7M8.5 14.5h7",
        ),
        "folder-plus" => Some("M3 7V4h6l2 3h10v13H3zM12 10v7M8.5 13.5h7"),
        "bookmark" => Some("M6 3h12v18l-6-4-6 4z"),
        "network" => Some("M5 5h4v4H5zM16 15h4v4h-4zM3 17h4v4H3zM9 8l7 7M7 9l-2 8M7 19l9-2"),
        "arrow-up-down" => Some("M8 3v18M4 7l4-4 4 4M16 21V3M12 17l4 4 4-4"),
        "locate" => Some("M12 2v4M12 18v4M2 12h4M18 12h4M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0"),
        "fold-vertical" => Some("M4 12h16M8 3l4 4 4-4M12 1v6M8 21l4-4 4 4M12 17v6"),
        "list" => Some("M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"),
        "link" | "links" => Some(
            "M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-2 2M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l2-2",
        ),
        "tags" => Some("M3 3h8l10 10-8 8L3 11zM7 7h.01"),
        "pencil" => Some("M16 3l5 5-13 13H3v-5zM13 6l5 5"),
        _ => None,
    };
    if let Some(shape) = shape {
        return Icon::default().data(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="{shape}"/></svg>"#).as_bytes()).size(px(17.));
    }
    Icon::default()
        .path(format!("icons/{name}.svg"))
        .size(px(17.))
}
fn tool(id: &'static str, name: &str, tip: &'static str) -> Button {
    Button::new(id)
        .accessibility_id(id)
        .accessibility_label(tip)
        .ghost()
        .compact()
        .icon(icon(name))
        .tooltip(tip)
        .w(px(30.))
        .h(px(28.))
}
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
    (16, "查看回收站", ""),
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
    (28, "打开今天的日记", ""),
    (29, "插入模板", ""),
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
    (43, "打开关系图谱", ""),
    (44, "打开当前笔记的局部关系图", ""),
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
    (75, "插入当前日期", ""),
    (76, "插入当前时间", ""),
    (77, "打开上一篇日记", ""),
    (78, "打开下一篇日记", ""),
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
    (97, "显示 / 隐藏功能区", ""),
    (98, "切换注释", "Ctrl+/"),
];

#[derive(Clone)]
struct DraggedRibbonAction {
    id: usize,
    label: String,
}
impl Render for DraggedRibbonAction {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(0x363636))
            .text_color(rgb(0xdddddd))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
struct DraggedTab {
    id: usize,
    label: String,
}
impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(0x363636))
            .text_color(rgb(0xdddddd))
            .child(self.label.clone())
    }
}

impl Workspace {
    fn bg(&self) -> Rgba {
        rgb(if self.ui.prefs.light {
            0xffffff
        } else {
            0x262626
        })
    }
    pub(super) fn side(&self) -> Rgba {
        rgb(if self.ui.prefs.light {
            0xf6f6f6
        } else {
            0x212121
        })
    }
    fn fg(&self) -> Rgba {
        rgb(if self.ui.prefs.light {
            0x0f0f0f
        } else {
            0xd1d1d1
        })
    }
    pub(super) fn border(&self) -> Rgba {
        rgb(if self.ui.prefs.light {
            0xe6e6e6
        } else {
            0x363636
        })
    }
    pub(super) fn persist_workspace(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.snapshot_views(cx);
        let Some(vault) = &self.vault else {
            return;
        };
        if self.loading || self.ui.persisting {
            return;
        }
        self.ui.prefs.open_paths = self.tabs.iter().map(|t| t.path.clone()).collect();
        self.ui.prefs.left_panel = self.ui.left_mode;
        self.ui.prefs.right_panel = self.ui.right_mode;
        self.ui.prefs.active_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        self.ui.prefs.active_tab_index = self.active;
        let Ok(serialized) = serde_json::to_string(&self.ui.prefs) else {
            return;
        };
        if serialized == self.ui.last_persisted {
            return;
        }
        self.ui.persisting = true;
        let prefs = self.ui.prefs.clone();
        let path = vault.root.join(".inkstone-workspace.json");
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { prefs.save(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.persisting = false;
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.ui.last_persisted = serialized;
                        this.persist_workspace(cx);
                    }
                    Err(e) => {
                        this.status = format!("无法保存工作区设置：{e}");
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn apply_editor_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_font_preferences(cx);
        self.sync_font_selects(window, cx);
        let text_font = self.resolved_font(&self.ui.prefs.text_font, "Microsoft YaHei UI");
        let p = self.ui.prefs.clone();
        self.ui
            .font_size_slider
            .update(cx, |slider, cx| slider.set_value(p.font_size, window, cx));
        self.ui.tab_width.update(cx, |slider, cx| {
            slider.set_value(p.tab_size as f32, window, cx);
        });
        if let Some(graph) = &self.graph {
            graph.update(cx, |g, cx| g.set_index(self.index.clone(), p.light, cx));
        }
        let panes: Vec<_> = self
            .tabs
            .iter()
            .map(|t| t.pane.clone())
            .chain(self.views.split.iter().map(|s| s.pane.clone()))
            .collect();
        for pane in panes {
            pane.update(cx, |pane, cx| {
                pane.font_size = p.font_size;
                pane.quick_font_size = p.quick_font_size;
                pane.text_font = text_font.clone();
                pane.readable_width = p.readable_width;
                pane.strict_line_breaks = p.strict_line_breaks;
                pane.light = p.light;
                pane.smart_lists = p.smart_lists;
                pane.set_fold_options(p.fold_headings, p.fold_indentation, window, cx);
                pane.set_auto_pairing(p.auto_pair_brackets, p.auto_pair_markdown, cx);
                pane.indentation = gpui_base::input::TabSize {
                    tab_size: p.tab_size.clamp(2, 8),
                    hard_tabs: p.use_tabs,
                };
                pane.editor.update(cx, |editor, cx| {
                    editor.set_line_number(p.line_numbers, window, cx);
                    editor.set_indent_guides(p.show_indent_guides, window, cx);
                    editor.set_tab_size(pane.indentation, cx);
                });
                cx.notify();
            });
        }
        self.persist_workspace(cx);
        cx.notify();
    }
    pub(super) fn set_default_view(&mut self, reading: bool, cx: &mut Context<Self>) {
        self.ui.prefs.default_reading = reading;
        self.persist_workspace(cx);
        cx.notify();
    }
    pub(super) fn set_default_editing_mode(&mut self, live: bool, cx: &mut Context<Self>) {
        self.ui.prefs.default_live_preview = live;
        let mut panes: Vec<_> = self.tabs.iter().map(|tab| tab.pane.clone()).collect();
        if let Some(split) = &self.views.split {
            panes.push(split.pane.clone());
        }
        for pane in panes {
            pane.update(cx, |pane, cx| {
                pane.live = live;
                cx.notify();
            });
        }
        self.persist_workspace(cx);
        cx.notify();
    }
    fn filtered_commands(&self, cx: &App) -> Vec<(usize, &'static str, &'static str)> {
        let query = self.ui.command.read(cx).value().to_lowercase();
        COMMANDS
            .iter()
            .copied()
            .filter(|(_, text, shortcut)| {
                text.to_lowercase().contains(&query) || shortcut.to_lowercase().contains(&query)
            })
            .collect()
    }
    fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_open = true;
        self.ui.more = false;
        self.ui.selected = 0;
        self.ui.command.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }
    fn prompt_name(&mut self, mut mode: NameMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == NameMode::Rename
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path.as_os_str().is_empty())
        {
            mode = NameMode::New;
        }
        self.ui.name_mode = Some(mode);
        self.ui.more = false;
        let name = if mode == NameMode::Rename {
            self.active
                .and_then(|i| self.tabs.get(i))
                .map(|t| t.path.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        self.name.update(cx, |s, cx| {
            s.set_value(name, window, cx);
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.notify();
    }
    pub(super) fn submit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation || self.ui.link_update.is_some() {
            return;
        }
        match self.ui.name_mode.take().unwrap_or(NameMode::New) {
            NameMode::New => self.create_note(window, cx),
            NameMode::Rename => self.manage_note(false, window, cx),
            NameMode::RenameFolder => {
                if let Some(old) = self.ui.folder_target.take() {
                    let new = PathBuf::from(self.name.read(cx).value().as_ref());
                    self.manage_folder(old, Some(new), window, cx);
                }
            }
            NameMode::Folder => {
                let Some(vault) = self.vault.clone() else {
                    return;
                };
                let name = self.name.read(cx).value().to_string();
                let generation = self.generation;
                self.ui.pending_file_writes += 1;
                let task = cx
                    .background_executor()
                    .spawn(async move { vault.create_folder(std::path::Path::new(&name)) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                        if this.generation != generation {
                            return;
                        }
                        this.status = match result {
                            Ok(()) => "文件夹已创建".into(),
                            Err(e) => e.to_string(),
                        };
                        this.refresh_requested = true;
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        self.focus_after_link_update(window, cx);
        cx.notify();
    }
    fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.current_pane() else {
            return;
        };
        let cursor = pane.read(cx).navigation.cursor;
        let target = if forward {
            cursor.checked_add(1)
        } else {
            cursor.checked_sub(1)
        };
        if let Some(target) = target {
            self.navigate_to(target, window, cx);
        }
    }

    pub(super) fn navigate_to(
        &mut self,
        target: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut history = self.navigation_with_current_state(cx);
        if target == history.cursor {
            return;
        }
        let Some(entry) = history.entries.get(target) else {
            return;
        };
        let path = entry.path.clone();
        history.cursor = target;
        self.navigation_generation += 1;
        self.pending_navigation = None;
        self.pending_jump = None;
        self.open_current_note(path, history, window, cx);
    }

    fn navigate_to_new_tab(&mut self, target: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.current_pane() else {
            return;
        };
        if source.read(cx).navigation.entries.get(target).is_none() {
            return;
        }
        self.open_current_note_in_new_tab(window, cx);
        if self.current_pane().is_some_and(|pane| pane != source) {
            self.navigate_to(target, window, cx);
        }
    }

    fn focus_navigation_pane(
        &mut self,
        pane: &Entity<EditorPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|split| split.pane == *pane)
        {
            self.focus_secondary(cx);
            true
        } else if let Some(index) = self.tabs.iter().position(|tab| tab.pane == *pane) {
            self.focus_primary(index, window, cx);
            true
        } else {
            false
        }
    }

    fn navigation_button(
        &self,
        pane: Option<Entity<EditorPane>>,
        forward: bool,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries: Vec<_> = pane
            .as_ref()
            .map(|pane| {
                let history = &pane.read(cx).navigation;
                let mut entries: Vec<_> = history
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        if forward {
                            *index > history.cursor
                        } else {
                            *index < history.cursor
                        }
                    })
                    .map(|(index, entry)| (index, entry.path.clone()))
                    .collect();
                if !forward {
                    entries.reverse();
                }
                entries
            })
            .unwrap_or_default();
        let disabled = entries.is_empty();
        let cursor = pane.as_ref().map(|pane| pane.read(cx).navigation.cursor);
        let click_pane = pane.clone();
        let (id, name, tip) = if forward {
            ("forward", "arrow-right", "前进 Alt+Right")
        } else {
            ("back", "arrow-left", "返回 Alt+Left")
        };
        let button = div()
            .id(("navigation-button", usize::from(forward)))
            .debug_selector(move || {
                format!(
                    "{}-history-{id}",
                    if secondary { "secondary" } else { "main" }
                )
            })
            .child(tool(id, name, tip).disabled(disabled).on_click(cx.listener(
                move |this, event: &ClickEvent, window, cx| {
                    if let Some(pane) = &click_pane
                        && this.focus_navigation_pane(pane, window, cx)
                    {
                        if event.modifiers().secondary() {
                            let cursor = pane.read(cx).navigation.cursor;
                            let target = if forward {
                                cursor.checked_add(1)
                            } else {
                                cursor.checked_sub(1)
                            };
                            if let Some(target) = target {
                                this.navigate_to_new_tab(target, window, cx);
                            }
                        } else {
                            this.navigate(forward, window, cx);
                        }
                    }
                },
            )));
        if disabled {
            return button.into_any_element();
        }
        let weak = cx.entity().downgrade();
        button
            .context_menu(move |mut menu, _, _| {
                for (target, path) in &entries {
                    let target = *target;
                    let path = path.clone();
                    let label: String = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(50)
                        .collect();
                    let weak = weak.clone();
                    let pane = pane.clone();
                    menu = menu.item(PopupMenuItem::new(label).icon(icon("file")).on_click(
                        move |event, window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                let Some(pane) = &pane else { return };
                                let history = &pane.read(cx).navigation;
                                if Some(history.cursor) != cursor
                                    || history
                                        .entries
                                        .get(target)
                                        .is_none_or(|entry| entry.path != path)
                                {
                                    return;
                                }
                                if this.focus_navigation_pane(pane, window, cx) {
                                    if event.modifiers().secondary() {
                                        this.navigate_to_new_tab(target, window, cx);
                                    } else {
                                        this.navigate_to(target, window, cx);
                                    }
                                }
                            });
                        },
                    ));
                }
                menu
            })
            .long_press(Duration::from_millis(400))
            .into_any_element()
    }
    fn cycle_tab(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
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
                    inkstone::markdown_edit::inline_format(&text, range, left, right)
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
        use inkstone::markdown_edit::BlockFormat;
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
                    if let Some(edit) = inkstone::markdown_edit::block_format(
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
                    if let Some(edit) =
                        inkstone::markdown_edit::insert_footnote(&s.value(), s.selected_range())
                        && s.apply_source_edit(
                            edit.range,
                            &edit.replacement,
                            edit.selection,
                            window,
                            cx,
                        )
                    {
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
                    if let Some(edit) =
                        inkstone::markdown_edit::insert_link(&s.value(), s.selected_range(), wiki)
                    {
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
        use inkstone::tables::Operation;
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
                        inkstone::tables::edit(&s.value(), s.selected_range(), operation)
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
            self.status =
                "请在独立表格的单元格中操作，并完成输入法组词；结构编辑暂支持最多 10 万个单元格。"
                    .into();
        }
    }
    pub(super) fn execute_command(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.graph_open
            && matches!(id, 6 | 7 | 23..=26 | 35..=38 | 45..=67 | 71..=73 | 79..=84 | 95..=96 | 98)
        {
            self.graph_open = false;
        }
        self.command_open = false;
        self.ui.more = false;
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
            12 => self.ui.prefs.left_open = !self.ui.prefs.left_open,
            13 => self.ui.prefs.right_open = !self.ui.prefs.right_open,
            14 => {
                self.prepare_file_settings(window, cx);
                self.prepare_daily_settings(window, cx);
                self.prepare_template_settings(window, cx);
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
            16 => {
                self.ui.trash_open = true;
                window.focus(&self.ui.modal_focus, cx);
                self.refresh_trash(cx);
            }
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
                    inkstone::preferences::ThemeMode::Dark
                } else {
                    inkstone::preferences::ThemeMode::Light
                };
                self.set_theme(theme, window, cx);
            }
            23 => {
                if let Some(t) = self.active.and_then(|i| self.tabs.get(i)) {
                    t.pane.read(cx).editor.clone().update(cx, |s, cx| {
                        s.open_search(true, cx);
                        s.focus(window, cx);
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
            28 => self.open_daily(window, cx),
            29 => self.open_template_picker(window, cx),
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
            43 => self.open_graph(false, window, cx),
            44 => self.open_graph(true, window, cx),
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
            75..=76 => self.insert_current_date_time(id == 76, window, cx),
            77..=78 => self.open_neighboring_daily(id == 78, window, cx),
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
            97 => self.ui.prefs.show_ribbon = !self.ui.prefs.show_ribbon,
            98 => {
                if let Some(pane) = self.current_pane() {
                    pane.update(cx, |pane, cx| {
                        if !pane.reading {
                            pane.editor.update(cx, |editor, cx| {
                                editor.apply_selection_transform(
                                    inkstone::comments::toggle,
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
    fn refresh_trash(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { vault.trash_entries() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(entries) => this.ui.trash = entries,
                    Err(e) => this.status = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn restore_deleted(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(entry) = self.ui.trash.get(i).cloned() else {
            return;
        };
        let generation = self.generation;
        self.ui.pending_file_writes += 1;
        let task = cx
            .background_executor()
            .spawn(async move { vault.restore_trash(&entry) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.status = match result {
                    Ok(()) => "文件已恢复到原目录".into(),
                    Err(e) => e.to_string(),
                };
                this.refresh_requested = true;
                this.refresh_trash(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.link_update.take().is_some() {
            self.status = "文件已移动，链接保持原样。".into();
            self.focus_after_link_update(window, cx);
            cx.notify();
            return;
        }
        self.command_open = false;
        self.close_quick_search(window, cx);
        self.ui.template_mode = false;
        self.ui.name_mode = None;
        self.ui.settings = false;
        self.ui.hotkey_recording = None;
        self.ui.property_open = false;
        self.ui.property_error.clear();
        self.ui.property_baseline = None;
        self.ui.property_original = None;
        self.ui.more = false;
        self.ui.trash_open = false;
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        } else {
            window.focus(&self.ui.workspace_focus, cx);
        }
        cx.notify();
    }

    pub(super) fn set_ribbon_command(
        &mut self,
        command: usize,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !inkstone::preferences::RIBBON_COMMANDS
            .iter()
            .any(|item| item.0 == command)
        {
            return;
        }
        if enabled {
            if !self.ui.prefs.ribbon_commands.contains(&command) {
                self.ui.prefs.ribbon_commands.push(command);
            }
        } else {
            self.ui.prefs.ribbon_commands.retain(|id| *id != command);
        }
        self.persist_workspace(cx);
        cx.notify();
    }

    fn move_ribbon_command(&mut self, command: usize, target: usize, cx: &mut Context<Self>) {
        let Some(from) = self
            .ui
            .prefs
            .ribbon_commands
            .iter()
            .position(|id| *id == command)
        else {
            return;
        };
        let Some(to) = self
            .ui
            .prefs
            .ribbon_commands
            .iter()
            .position(|id| *id == target)
        else {
            return;
        };
        let command = self.ui.prefs.ribbon_commands.remove(from);
        self.ui.prefs.ribbon_commands.insert(to, command);
        self.persist_workspace(cx);
        cx.notify();
    }

    pub(super) fn ribbon_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut order = self.ui.prefs.ribbon_commands.clone();
        for &(id, _, _, _) in inkstone::preferences::RIBBON_COMMANDS {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        let rows = order
            .into_iter()
            .filter_map(|command| {
                inkstone::preferences::RIBBON_COMMANDS
                    .iter()
                    .find(|item| item.0 == command)
            })
            .enumerate()
            .map(|(index, &(command, _, _, label))| {
                let enabled = self.ui.prefs.ribbon_commands.contains(&command);
                div()
                    .id(("ribbon-config", command))
                    .when(enabled, |row| {
                        row.on_drag(
                            DraggedRibbonAction {
                                id: command,
                                label: label.into(),
                            },
                            |drag, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            },
                        )
                        .on_drop(cx.listener(
                            move |this, drag: &DraggedRibbonAction, _, cx| {
                                this.move_ribbon_command(drag.id, command, cx)
                            },
                        ))
                    })
                    .child(
                        self.settings_row(
                            label,
                            if enabled {
                                "拖动可调整显示顺序。"
                            } else {
                                "开启后在功能区显示。"
                            },
                            setting_switch(("ribbon-toggle", command))
                                .accessibility_label(label)
                                .checked(enabled)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.set_ribbon_command(command, *checked, cx)
                                })),
                            index > 0,
                            20.,
                        ),
                    )
                    .into_any_element()
            })
            .collect();
        self.settings_group("功能区按钮", rows)
    }

    fn ribbon(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("workspace-ribbon")
            .debug_selector(|| "workspace-ribbon".into())
            .w(px(44.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .py_2()
            .gap_1()
            .bg(self.side())
            .border_r_1()
            .border_color(self.border())
            .children(
                self.ui
                    .prefs
                    .ribbon_commands
                    .iter()
                    .filter_map(|command| {
                        inkstone::preferences::RIBBON_COMMANDS
                            .iter()
                            .find(|item| item.0 == *command)
                    })
                    .map(|&(command, id, icon, label)| {
                        div()
                            .id(("ribbon-drag", command))
                            .debug_selector(move || format!("ribbon-action-{command}"))
                            .on_drag(
                                DraggedRibbonAction {
                                    id: command,
                                    label: label.into(),
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            )
                            .on_drop(cx.listener(move |this, drag: &DraggedRibbonAction, _, cx| {
                                this.move_ribbon_command(drag.id, command, cx)
                            }))
                            .child(tool(id, icon, label).w(px(32.)).h(px(32.)).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.execute_command(command, window, cx)
                                }),
                            ))
                    }),
            )
            .child(div().flex_1())
            .child(
                tool("settings", "settings", "设置")
                    .w(px(32.))
                    .h(px(32.))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.execute_command(14, window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn left_header(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .h(px(40.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(self.border())
            .child(
                tool("files", "folder-closed", "文件列表")
                    .selected(self.ui.left_mode == 0)
                    .toggled(self.ui.left_mode == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.left_mode = 0;
                        cx.notify();
                    })),
            )
            .child(
                tool("search", "search", "搜索")
                    .selected(self.ui.left_mode == 1)
                    .toggled(self.ui.left_mode == 1)
                    .on_click(cx.listener(|this, _, w, cx| this.focus_search(true, w, cx))),
            )
            .child(
                tool("bookmarks", "bookmark", "书签")
                    .selected(self.ui.left_mode == 2)
                    .toggled(self.ui.left_mode == 2)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.left_mode = 2;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                tool("left-hide", "panel-left-close", "收起左侧栏")
                    .on_click(cx.listener(|this, _, w, cx| this.execute_command(12, w, cx))),
            )
            .into_any_element()
    }
    fn right_header(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .h(px(40.))
            .px_1()
            .gap_0()
            .border_b_1()
            .border_color(self.border())
            .children(
                [
                    (1, "links", "反向链接"),
                    (2, "link", "出链"),
                    (3, "tags", "标签"),
                    (4, "inbox", "属性"),
                    (0, "list", "大纲"),
                ]
                .iter()
                .map(|&(i, ico, label)| {
                    Button::new(("right-mode", i))
                        .accessibility_id(format!("right-mode-{i}"))
                        .accessibility_label(label)
                        .selected(self.ui.right_mode == i)
                        .toggled(self.ui.right_mode == i)
                        .ghost()
                        .compact()
                        .icon(icon(ico))
                        .tooltip(label)
                        .w(px(32.))
                        .h(px(28.))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.ui.right_mode = i;
                            if i == 3 {
                                w.focus(&this.ui.tags_focus, cx);
                            }
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }
    fn tab_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let menu_tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|tab| {
                let name = if tab.path.as_os_str().is_empty() {
                    "新标签页".into()
                } else {
                    tab.path.to_string_lossy().replace('\\', "/")
                };
                (
                    tab.id,
                    format!("{name}{}", if tab.save.dirty.get() { " •" } else { "" }),
                    tab.pinned,
                )
            })
            .collect();
        let selected = self
            .main_tab()
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.id);
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .w_full()
            .min_w_0()
            .h(px(40.))
            .items_center()
            .bg(self.side())
            .border_b_1()
            .border_color(self.border())
            .when(!self.ui.prefs.left_open, |s| {
                s.child(
                    tool("show-left", "panel-left", "展开左侧栏")
                        .on_click(cx.listener(|this, _, w, cx| this.execute_command(12, w, cx))),
                )
            })
            .child(
                div()
                    .id("tabs")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.ui.tab_scroll)
                    .items_end()
                    .px_1()
                    .gap_0()
                    .children(self.tabs.iter().enumerate().map(|(i, t)| {
                        let tab_id = t.id;
                        let menu_weak = cx.entity().downgrade();
                        let selected = self.main_tab() == Some(i);
                        let pinned = t.pinned;
                        div()
                            .id(("tab", t.id))
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_down(
                                MouseButton::Middle,
                                cx.listener(move |this, _, window, cx| {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.ui.middle_pressed_tab = Some(tab_id);
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Middle,
                                cx.listener(move |this, _, _, _| {
                                    if this.ui.middle_pressed_tab == Some(tab_id) {
                                        this.ui.middle_pressed_tab = None;
                                    }
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Middle,
                                cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    if this.ui.middle_pressed_tab.take() == Some(tab_id)
                                        && let Some(index) =
                                            this.tabs.iter().position(|tab| tab.id == tab_id)
                                    {
                                        this.close_tab_at(index, window, cx);
                                    }
                                }),
                            )
                            .on_drag(
                                DraggedTab {
                                    id: t.id,
                                    label: t
                                        .path
                                        .file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            )
                            .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| {
                                if let Some(from) = this.tabs.iter().position(|t| t.id == drag.id) {
                                    let active_id =
                                        this.active.and_then(|i| this.tabs.get(i)).map(|t| t.id);
                                    let tab = this.tabs.remove(from);
                                    this.tabs.insert(i.min(this.tabs.len()), tab);
                                    this.active = active_id
                                        .and_then(|id| this.tabs.iter().position(|t| t.id == id));
                                    this.persist_workspace(cx);
                                    cx.notify();
                                }
                            }))
                            .flex()
                            .relative()
                            .group(format!("note-tab-{}", t.id))
                            .items_center()
                            .h(px(34.))
                            .w(px(200.))
                            .min_w(px(100.))
                            .max_w(px(320.))
                            .px_2()
                            .gap_1()
                            .rounded_t(px(6.))
                            .text_color(if selected {
                                self.fg()
                            } else {
                                rgb(if self.ui.prefs.light {
                                    0x5c5c5c
                                } else {
                                    0x999999
                                })
                            })
                            .cursor_pointer()
                            .when(!selected, |s| s.hover(|s| s.bg(rgba(0x88888818))))
                            .when(selected, |s| {
                                s.bg(self.bg())
                                    .border_1()
                                    .border_b_0()
                                    .border_color(self.border())
                            })
                            .child(div().truncate().flex_1().text_size(px(13.)).child(format!(
                                "{}{}",
                                if t.path.as_os_str().is_empty() {
                                    "新标签页".into()
                                } else {
                                    t.path.file_stem().unwrap_or_default().to_string_lossy()
                                },
                                if t.save.conflict.get() || t.save.error.borrow().is_some() {
                                    " ⚠"
                                } else if t.save.dirty.get() {
                                    " •"
                                } else {
                                    ""
                                }
                            )))
                            .child(
                                Button::new(("close-tab", t.id))
                                    .accessibility_id(format!("close-tab-{}", t.id))
                                    .accessibility_label(format!(
                                        "{} {}",
                                        if pinned {
                                            "取消固定标签页"
                                        } else {
                                            "关闭标签页"
                                        },
                                        if t.path.as_os_str().is_empty() {
                                            "新标签页".to_string()
                                        } else {
                                            t.path.to_string_lossy().to_string()
                                        }
                                    ))
                                    .ghost()
                                    .compact()
                                    .icon(icon(if pinned { "pin" } else { "x" }).size(px(14.)))
                                    .when(!selected && !pinned, |button| {
                                        button
                                            .opacity(0.)
                                            .group_hover(format!("note-tab-{}", t.id), |style| {
                                                style.opacity(1.)
                                            })
                                    })
                                    .w(px(22.))
                                    .h(px(22.))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        cx.stop_propagation();
                                        this.close_tab_at(i, w, cx);
                                    })),
                            )
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.focus_primary(i, w, cx)),
                            )
                            .context_menu(move |mut menu, _, _| {
                                let weak = menu_weak.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(if pinned {
                                        "取消固定"
                                    } else {
                                        "固定"
                                    })
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                if let Some(index) = this
                                                    .tabs
                                                    .iter()
                                                    .position(|tab| tab.id == tab_id)
                                                {
                                                    this.focus_primary(index, window, cx);
                                                    this.execute_command(27, window, cx);
                                                }
                                            });
                                        },
                                    ),
                                );
                                let weak = menu_weak.clone();
                                menu = menu.item(PopupMenuItem::new("在新标签页中打开").on_click(
                                    move |_, window, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            if let Some(index) =
                                                this.tabs.iter().position(|tab| tab.id == tab_id)
                                            {
                                                this.focus_primary(index, window, cx);
                                                this.execute_command(85, window, cx);
                                            }
                                        });
                                    },
                                ));
                                for (mode, label) in [
                                    (0, "关闭其他标签页"),
                                    (1, "关闭右侧标签页"),
                                    (2, "关闭所有未固定标签页"),
                                ] {
                                    let weak = menu_weak.clone();
                                    menu = menu.item(PopupMenuItem::new(label).on_click(
                                        move |_, w, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.close_tab_group(Some(tab_id), mode, w, cx)
                                            });
                                        },
                                    ));
                                }
                                menu
                            })
                    })),
            )
            .child(
                tool("add-tab", "plus", "新建标签页 Ctrl+T")
                    .on_click(cx.listener(|this, _, w, cx| this.new_blank(w, cx))),
            )
            .child(
                tool("list-tabs", "chevron-down", "显示所有标签页").dropdown_menu(
                    move |mut menu, _, _| {
                        for (id, label, pinned) in &menu_tabs {
                            let id = *id;
                            let weak = weak.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label.clone())
                                    .when(*pinned, |item| item.icon(icon("pin")))
                                    .checked(selected == Some(id))
                                    .on_click(move |_, window, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            if let Some(index) =
                                                this.tabs.iter().position(|tab| tab.id == id)
                                            {
                                                this.focus_primary(index, window, cx);
                                            }
                                        });
                                    }),
                            );
                        }
                        menu
                    },
                ),
            )
            .when(true, |s| {
                s.child(
                    tool("show-right", "panel-right", "展开右侧栏")
                        .on_click(cx.listener(|this, _, w, cx| this.execute_command(13, w, cx))),
                )
            })
            .into_any_element()
    }
    pub(super) fn reveal_current_file(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone())
            .filter(|p| !p.as_os_str().is_empty())
        else {
            return false;
        };
        let tree_path: PathBuf = path.components().collect();
        let id: SharedString = tree_path.to_string_lossy().to_string().into();
        let found = self.tree.update(cx, |tree, cx| {
            tree.reveal_item(&id, ScrollStrategy::Center, cx);
            if let Some(index) = tree.index_of(&id) {
                tree.set_selected_index(Some(index), cx);
                true
            } else {
                false
            }
        });
        if found {
            self.ui.last_revealed_file = Some((self.generation, path));
        }
        found
    }
    fn left_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let menu_weak = cx.entity().downgrade();
        let active_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.path.clone());
        let tree_foreground = rgb(if self.ui.prefs.light {
            0x5c5c5c
        } else {
            0xaaaaaa
        });
        let tree_active = rgb(if self.ui.prefs.light {
            0xe8e8e8
        } else {
            0x2e2e2e
        });
        let tree_guide = self.border();
        let file_tree = Tree::new(&self.tree, move |i, entry, _, _, _| {
            let path = PathBuf::from(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let weak = weak.clone();
            ListItem::new(i)
                .h(px(27.))
                .px_1()
                .py_0()
                .rounded(px(4.))
                .text_size(px(13.))
                .text_color(tree_foreground)
                .when(!folder && active_path.as_ref() == Some(&path), |s| {
                    s.bg(tree_active)
                })
                .accessibility_label(entry.item().label.clone())
                .children((0..entry.depth()).map(|depth| {
                    div()
                        .absolute()
                        .left(px(12. + depth as f32 * 17.))
                        .top_0()
                        .bottom_0()
                        .w(px(1.))
                        .bg(tree_guide)
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .w_full()
                        .pl(px(entry.depth() as f32 * 17.))
                        .child(if folder {
                            icon(if entry.is_expanded() {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            })
                            .size(px(16.))
                        } else {
                            Icon::default().size(px(16.))
                        })
                        .child(div().truncate().child(if folder {
                            entry.item().label.to_string()
                        } else {
                            entry.item().label.trim_end_matches(".md").to_owned()
                        })),
                )
                .on_click(move |_, window, cx| {
                    if !folder {
                        let _ =
                            weak.update(cx, |this, cx| this.open_note(path.clone(), window, cx));
                    }
                })
        })
        .context_menu(move |_, entry, mut menu, _, _| {
            use gpui_component::menu::PopupMenuItem;
            if entry.is_folder() {
                let path = entry.item().id.to_string();
                let weak = menu_weak.clone();
                menu = menu.item(PopupMenuItem::new("新建笔记").on_click(move |_, w, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.prompt_name(NameMode::New, w, cx);
                        this.name
                            .update(cx, |s, cx| s.set_value(format!("{path}/未命名.md"), w, cx));
                    });
                }));
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                menu = menu.item(PopupMenuItem::new("重命名文件夹 / 移动文件夹").on_click(
                    move |_, w, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.ui.folder_target = Some(path.clone());
                            this.prompt_name(NameMode::RenameFolder, w, cx);
                            this.name.update(cx, |s, cx| {
                                s.set_value(path.to_string_lossy().to_string(), w, cx)
                            });
                        });
                    },
                ));
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                return menu.item(PopupMenuItem::new("将文件夹移入回收站").on_click(
                    move |_, w, cx| {
                        let _ = weak
                            .update(cx, |this, cx| this.manage_folder(path.clone(), None, w, cx));
                    },
                ));
            }

            for id in [8, 42, 15, 27, 11, 18, 19, 10] {
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(COMMANDS[id].1).on_click(move |_, w, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.ui.pending_command = Some((path.clone(), id));
                            this.open_note(path.clone(), w, cx);
                        });
                    }),
                );
            }
            menu
        });
        div()
            .id("workspace-left-panel")
            .debug_selector(|| "workspace-left-panel".into())
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(self.side())
            .when(self.ui.left_mode == 0, |s| {
                s.child(
                    div()
                        .flex()
                        .h(px(38.))
                        .items_center()
                        .px_2()
                        .gap_1()
                        .child(
                            tool("new-file", "square-pen", "新建笔记")
                                .on_click(cx.listener(|this, _, w, cx| this.focus_new(w, cx))),
                        )
                        .child(tool("new-folder", "folder-plus", "新建文件夹").on_click(
                            cx.listener(|this, _, w, cx| this.prompt_name(NameMode::Folder, w, cx)),
                        ))
                        .child({
                            let weak = cx.entity().downgrade();
                            let selected = (self.ui.prefs.sort_by, self.ui.prefs.sort_descending);
                            tool("sort", "arrow-up-down", "更改排序方式").dropdown_menu(
                                move |mut menu, _, _| {
                                    for (label, by, descending) in [
                                        ("文件名：A → Z", SortBy::Name, false),
                                        ("文件名：Z → A", SortBy::Name, true),
                                        ("修改时间：从新到旧", SortBy::Modified, true),
                                        ("修改时间：从旧到新", SortBy::Modified, false),
                                        ("创建时间：从新到旧", SortBy::Created, true),
                                        ("创建时间：从旧到新", SortBy::Created, false),
                                    ] {
                                        let weak = weak.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(selected == (by, descending))
                                                .on_click(move |_, _, cx| {
                                                    let _ = weak.update(cx, |this, cx| {
                                                        this.ui.prefs.sort_by = by;
                                                        this.ui.prefs.sort_descending = descending;
                                                        this.rebuild_sorted_tree(cx);
                                                        this.persist_workspace(cx);
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                },
                            )
                        })
                        .child(
                            tool("reveal", "locate", "自动显示当前文件")
                                .when(self.ui.prefs.auto_reveal_file, |s| s.bg(self.bg()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.ui.prefs.auto_reveal_file =
                                        !this.ui.prefs.auto_reveal_file;
                                    this.ui.last_revealed_file = None;
                                    if this.ui.prefs.auto_reveal_file {
                                        this.reveal_current_file(cx);
                                    }
                                    this.persist_workspace(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            tool("collapse", "fold-vertical", "折叠所有文件夹").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.ui.prefs.expanded_folders.clear();
                                    this.rebuild_sorted_tree(cx);
                                }),
                            ),
                        ),
                )
                .child(div().flex_1().min_h_0().px_3().child(file_tree))
            })
            .when(self.ui.left_mode == 1, |s| {
                s.when(!self.ui.quick_open, |s| {
                    s.child(
                        div()
                            .p_3()
                            .flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.search).cleanable(true)),
                            )
                            .child(
                                Button::new("search-case-sensitive")
                                    .ghost()
                                    .compact()
                                    .label("Aa")
                                    .accessibility_label("区分大小写")
                                    .tooltip("区分大小写")
                                    .toggled(self.ui.prefs.search_case_sensitive)
                                    .selected(self.ui.prefs.search_case_sensitive)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.ui.prefs.search_case_sensitive =
                                            !this.ui.prefs.search_case_sensitive;
                                        this.run_search(cx);
                                        this.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(rgb(0x999999))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(format!(
                            "已载入 {} 个文件 · {} 行",
                            self.search_results
                                .iter()
                                .map(|hit| &hit.path)
                                .collect::<std::collections::BTreeSet<_>>()
                                .len(),
                            self.search_results.len()
                        ))
                        .child(self.search_sort_button(cx))
                        .child(
                            tool("search-collapse", "fold-vertical", "展开或折叠全部搜索结果")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let paths: std::collections::BTreeSet<_> = this
                                        .search_results
                                        .iter()
                                        .map(|hit| hit.path.clone())
                                        .collect();
                                    if paths.is_subset(&this.ui.search_collapsed) {
                                        this.ui.search_collapsed.clear();
                                    } else {
                                        this.ui.search_collapsed = paths;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .when(!self.ui.search_error.is_empty(), |s| {
                    s.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_sm()
                            .line_height(relative(1.4))
                            .whitespace_normal()
                            .text_color(rgb(if self.ui.prefs.light {
                                0xb42318
                            } else {
                                0xfda29b
                            }))
                            .child(self.ui.search_error.clone()),
                    )
                })
                .child(self.search_list(false, cx))
            })
            .when(self.ui.left_mode == 2, |s| {
                s.child(
                    div()
                        .id("bookmark-list")
                        .flex_1()
                        .overflow_y_scroll()
                        .p_2()
                        .child(div().p_2().text_sm().child("书签"))
                        .when(self.ui.prefs.bookmarks.is_empty(), |s| {
                            s.child(
                                div()
                                    .p_2()
                                    .text_color(rgb(0x888888))
                                    .child("使用文件菜单添加书签"),
                            )
                        })
                        .children(self.ui.prefs.bookmarks.iter().enumerate().map(|(i, p)| {
                            let path = p.clone();
                            div()
                                .id(("bookmark", i))
                                .p_2()
                                .cursor_pointer()
                                .child(
                                    p.file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                )
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.open_note(path.clone(), w, cx)
                                }))
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(38.))
                    .px_2()
                    .border_t_1()
                    .border_color(self.border())
                    .child(
                        Button::new("vault-switch")
                            .ghost()
                            .compact()
                            .label(
                                self.vault
                                    .as_ref()
                                    .map(|v| {
                                        v.root
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string()
                                    })
                                    .unwrap_or("打开笔记库".into()),
                            )
                            .on_click(cx.listener(|this, _, w, cx| this.choose_vault(w, cx))),
                    )
                    .child(div().flex_1())
                    .when(!self.ui.prefs.show_ribbon, |row| {
                        row.child(tool("settings", "settings", "设置").on_click(
                            cx.listener(|this, _, w, cx| this.execute_command(14, w, cx)),
                        ))
                    }),
            )
            .into_any_element()
    }
    pub(super) fn rebuild_sorted_tree(&mut self, cx: &mut Context<Self>) {
        let mut paths = self.tree_files.clone();
        paths.extend(self.ui.folders.iter().cloned());
        let mut items = make_tree(&paths);
        fn apply(
            items: &mut [TreeItem],
            folders: &[PathBuf],
            expanded: &[PathBuf],
            index: &Index,
            by: SortBy,
            descending: bool,
        ) {
            for item in items.iter_mut() {
                let folder = item.is_folder()
                    || folders
                        .iter()
                        .any(|p| p == std::path::Path::new(item.id.as_ref()));
                let open = expanded
                    .iter()
                    .any(|p| p == std::path::Path::new(item.id.as_ref()));
                *item = item.clone().folder(folder).expanded(open);
                apply(&mut item.children, folders, expanded, index, by, descending);
            }
            items.sort_by(|a, b| {
                let times = |item: &TreeItem| {
                    index
                        .notes
                        .get(std::path::Path::new(item.id.as_ref()))
                        .map(|n| n.times)
                        .unwrap_or_default()
                };
                inkstone::file_order::compare(
                    (&a.label, a.is_folder(), times(a)),
                    (&b.label, b.is_folder(), times(b)),
                    by,
                    descending,
                )
            });
        }
        apply(
            &mut items,
            &self.ui.folders,
            &self.ui.prefs.expanded_folders,
            &self.index,
            self.ui.prefs.sort_by,
            self.ui.prefs.sort_descending,
        );
        self.tree.update(cx, |tree, cx| {
            let selected = tree.selected_item().cloned();
            tree.set_items(items, cx);
            tree.set_selected_item(selected.as_ref(), cx);
        });
        cx.notify();
    }
    pub(super) fn open_selected_result(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let hit = self
            .visible_search_hits(self.ui.quick_open, cx)
            .get(self.ui.selected)
            .map(|h| (h.path.clone(), h.offset));
        if let Some((path, offset)) = hit {
            if self.ui.template_mode {
                self.insert_template(&path, w, cx);
                return;
            }
            self.pending_jump = Some((path.clone(), offset));
            self.open_note(path, w, cx);
            self.apply_jump(w, cx);
        }
    }
    fn visible_search_hits(&self, modal: bool, cx: &Context<Self>) -> Vec<SearchHit> {
        if modal && self.ui.template_mode {
            return self.search_results.clone();
        }
        if modal && self.search.read(cx).value().is_empty() {
            self.files
                .iter()
                .take(100)
                .map(|p| SearchHit {
                    path: p.clone(),
                    display_name: None,
                    offset: 0,
                    line: 1,
                    excerpt: String::new(),
                    highlights: vec![],
                    title_highlights: vec![],
                })
                .collect()
        } else {
            self.search_results.clone()
        }
    }
    fn search_list(&self, modal: bool, cx: &mut Context<Self>) -> AnyElement {
        if !modal && self.fulltext {
            return self.grouped_search_results(cx);
        }
        let hits = self.visible_search_hits(modal, cx);
        let empty_templates = modal && self.ui.template_mode && hits.is_empty();
        div()
            .id(if modal {
                "quick-results"
            } else {
                "search-results"
            })
            .when(modal, |s| s.track_scroll(&self.ui.modal_scroll))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .when(empty_templates, |s| {
                s.child(div().p_3().text_color(rgb(0x999999)).child("未找到模板"))
            })
            .children(hits.into_iter().enumerate().map(|(i, hit)| {
                let path = hit.path;
                let offset = hit.offset;
                div()
                    .id(("result", i))
                    .when(modal && self.ui.selected == i, |s| s.bg(rgba(0x88888822)))
                    .p_2()
                    .when(modal, |s| s.py(px(6.)).px_3().h(px(33.)).overflow_hidden())
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0x88888822)))
                    .child(div().text_size(px(14.)).truncate().child(
                        hit.display_name.unwrap_or_else(|| {
                            if modal {
                                return path
                                    .with_extension("")
                                    .to_string_lossy()
                                    .replace('\\', "/");
                            }
                            path.file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        }),
                    ))
                    .when(!modal, |s| {
                        s.child(div().text_xs().text_color(rgb(0x888888)).truncate().child(
                            if hit.excerpt.is_empty() {
                                path.to_string_lossy().to_string()
                            } else {
                                format!("{}: {}", hit.line, hit.excerpt)
                            },
                        ))
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if this.ui.template_mode {
                            this.insert_template(&path, w, cx);
                            return;
                        }
                        this.pending_jump = Some((path.clone(), offset));
                        this.open_note(path.clone(), w, cx);
                        this.apply_jump(w, cx);
                    }))
            }))
            .into_any_element()
    }

    fn search_sort_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = (
            self.ui.prefs.search_sort_by,
            self.ui.prefs.search_descending,
        );
        let weak = cx.entity().downgrade();
        tool("search-sort", "arrow-up-down", "搜索结果排序")
            .dropdown_menu(move |mut menu, _, _| {
                for (by, descending, label) in [
                    (SortBy::Name, false, "文件名（升序）"),
                    (SortBy::Name, true, "文件名（降序）"),
                    (SortBy::Modified, true, "修改时间（从新到旧）"),
                    (SortBy::Modified, false, "修改时间（从旧到新）"),
                    (SortBy::Created, true, "创建时间（从新到旧）"),
                    (SortBy::Created, false, "创建时间（从旧到新）"),
                ] {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(current == (by, descending))
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.ui.prefs.search_sort_by = by;
                                    this.ui.prefs.search_descending = descending;
                                    this.ui.search_group_scroll.set_offset(Point::default());
                                    this.run_search(cx);
                                    this.persist_workspace(cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn grouped_search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut groups: Vec<(PathBuf, Vec<SearchHit>)> = vec![];
        let mut positions = std::collections::HashMap::new();
        for hit in &self.search_results {
            let position = *positions.entry(hit.path.clone()).or_insert_with(|| {
                groups.push((hit.path.clone(), vec![]));
                groups.len() - 1
            });
            groups[position].1.push(hit.clone());
        }
        div()
            .id("grouped-search-results")
            .track_scroll(&self.ui.search_group_scroll)
            .on_scroll_wheel(cx.listener(|_, _, w, cx| {
                cx.defer_in(w, |this, _, cx| {
                    let scroll = &this.ui.search_group_scroll;
                    if scroll.max_offset().y + scroll.offset().y <= px(120.) {
                        this.load_more_search(cx);
                    }
                });
            }))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .when(
                groups.is_empty()
                    && !self.ui.search_loading
                    && self.ui.search_error.is_empty()
                    && !self.search.read(cx).value().trim().is_empty(),
                |s| {
                    s.child(
                        div()
                            .p_2()
                            .text_sm()
                            .text_color(rgb(0x999999))
                            .child("未找到匹配结果"),
                    )
                },
            )
            .children(groups.into_iter().enumerate().map(|(i, (path, hits))| {
                let collapsed = self.ui.search_collapsed.contains(&path);
                let fold_path = path.clone();
                let open_path = path.clone();
                let offset = hits[0].offset;
                let title = path.to_string_lossy().replace('\\', "/");
                let title_highlights = hits[0].title_highlights.clone();
                div()
                    .id(("search-group", i))
                    .mb_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(("search-group-fold", i))
                                    .ghost()
                                    .compact()
                                    .w(px(20.))
                                    .label(if collapsed { "›" } else { "⌄" })
                                    .accessibility_label(format!("展开或折叠 {}", path.display()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.ui.search_collapsed.remove(&fold_path) {
                                            this.ui.search_collapsed.insert(fold_path.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("search-group-file", i))
                                    .ghost()
                                    .compact()
                                    .flex_1()
                                    .min_w_0()
                                    .justify_start()
                                    .overflow_hidden()
                                    .accessibility_label(format!("打开 {title}"))
                                    .child(StyledText::new(title).with_highlights(
                                        title_highlights.into_iter().map(|range| {
                                            (
                                                range,
                                                HighlightStyle {
                                                    background_color: Some(
                                                        rgba(if self.ui.prefs.light {
                                                            0xf4d03f66
                                                        } else {
                                                            0x9e7d2866
                                                        })
                                                        .into(),
                                                    ),
                                                    font_weight: Some(FontWeight::SEMIBOLD),
                                                    ..Default::default()
                                                },
                                            )
                                        }),
                                    ))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.pending_jump = Some((open_path.clone(), offset));
                                        this.open_note(open_path.clone(), w, cx);
                                        this.apply_jump(w, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x999999))
                                    .child(hits.len().to_string()),
                            ),
                    )
                    .children(hits.into_iter().filter(|_| !collapsed).enumerate().map(
                        |(j, hit)| {
                            let path = path.clone();
                            div()
                                .id(("search-line", j))
                                .pl(px(24.))
                                .py_1()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(0x88888822)))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x999999))
                                        .child(format!("第 {} 行", hit.line)),
                                )
                                .child(
                                    div().text_sm().whitespace_normal().child(
                                        StyledText::new(hit.excerpt.trim_end().to_string())
                                            .with_highlights(
                                                hit.highlights.iter().cloned().filter_map(
                                                    |mut range| {
                                                        range.end = range
                                                            .end
                                                            .min(hit.excerpt.trim_end().len());
                                                        if range.start >= range.end {
                                                            return None;
                                                        }
                                                        Some((
                                                            range,
                                                            HighlightStyle {
                                                                background_color: Some(
                                                                    rgba(if self.ui.prefs.light {
                                                                        0xf4d03f66
                                                                    } else {
                                                                        0x9e7d2866
                                                                    })
                                                                    .into(),
                                                                ),
                                                                font_weight: Some(
                                                                    FontWeight::SEMIBOLD,
                                                                ),
                                                                ..Default::default()
                                                            },
                                                        ))
                                                    },
                                                ),
                                            ),
                                    ),
                                )
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.pending_jump = Some((path.clone(), hit.offset));
                                    this.open_note(path.clone(), w, cx);
                                    this.apply_jump(w, cx);
                                }))
                        },
                    ))
            }))
            .when(
                self.ui.search_loading && self.search_results.is_empty(),
                |s| s.child(div().p_2().text_sm().child("正在搜索…")),
            )
            .when(self.ui.search_has_more, |s| {
                s.child(
                    Button::new("search-load-more")
                        .ghost()
                        .w_full()
                        .label(if self.ui.search_loading {
                            "正在载入…"
                        } else {
                            "显示更多结果"
                        })
                        .disabled(self.ui.search_loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load_more_search(cx))),
                )
            })
            .into_any_element()
    }

    fn right_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = self.current_pane();
        let headings = pane
            .as_ref()
            .map(|p| p.read(cx).parsed.headings.clone())
            .unwrap_or_default();
        let links = pane
            .as_ref()
            .map(|p| p.read(cx).parsed.links.clone())
            .unwrap_or_default();
        let properties = pane
            .as_ref()
            .map(|p| inkstone::properties::parse(&p.read(cx).editor.read(cx).value()))
            .unwrap_or_default();
        let tab_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        div()
            .id("workspace-right-panel")
            .debug_selector(|| "workspace-right-panel".into())
            .flex()
            .flex_col()
            .size_full()
            .bg(self.bg())
            .child(
                div()
                    .id("right-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .when(self.ui.right_mode == 0, |s| {
                        s.when(headings.is_empty(), |s| {
                            s.child(
                                div()
                                    .w_full()
                                    .pt(px(64.))
                                    .text_center()
                                    .text_color(rgb(0x777777))
                                    .child("未找到小标题行。"),
                            )
                        })
                        .children(headings.into_iter().enumerate().map(|(i, h)| {
                            let pane = pane.clone();
                            div()
                                .id(("outline", i))
                                .h(px(27.))
                                .text_size(px(13.))
                                .rounded(px(4.))
                                .hover(|s| s.bg(rgba(0x88888818)))
                                .py_1()
                                .pl(px(8. + (h.level - 1) as f32 * 17.))
                                .cursor_pointer()
                                .child(div().truncate().child(h.title))
                                .on_click(cx.listener(move |_, _, w, cx| {
                                    if let Some(pane) = &pane {
                                        pane.update(cx, |p, cx| p.jump(h.offset, w, cx));
                                    }
                                }))
                        }))
                    })
                    .when(self.ui.right_mode == 1, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(32.))
                                .text_size(px(12.))
                                .text_color(rgb(0x999999))
                                .child("链接当前文件")
                                .child(self.backlinks.len().to_string()),
                        )
                        .when(self.backlinks.is_empty(), |s| {
                            s.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(13.))
                                    .text_color(rgb(0x777777))
                                    .child("没有笔记链接当前文件"),
                            )
                        })
                        .child(
                            uniform_list(
                                "sidebar-backlinks",
                                self.backlinks.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .filter_map(|i| {
                                            this.backlinks.get(i).cloned().map(|path| {
                                                ListItem::new(("backlink", i))
                                                    .h(px(27.))
                                                    .px_2()
                                                    .text_size(px(13.))
                                                    .rounded(px(4.))
                                                    .child(
                                                        div().truncate().child(
                                                            path.to_string_lossy().to_string(),
                                                        ),
                                                    )
                                                    .on_click(cx.listener(move |this, _, w, cx| {
                                                        this.open_note(path.clone(), w, cx)
                                                    }))
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.backlink_scroll)
                            .h(px((self.backlinks.len() as f32 * 27.).min(400.)))
                            .w_full(),
                        )
                    })
                    .when(self.ui.right_mode == 2, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(32.))
                                .text_size(px(12.))
                                .text_color(rgb(0x999999))
                                .child("当前笔记中的链接")
                                .child(links.len().to_string()),
                        )
                        .children(links.into_iter().enumerate().map(|(i, link)| {
                            let from = tab_path.clone();
                            div()
                                .id(("outlink", i))
                                .h(px(27.))
                                .px_2()
                                .text_size(px(13.))
                                .rounded(px(4.))
                                .hover(|s| s.bg(rgba(0x88888818)))
                                .py_1()
                                .text_color(rgb(0x3f9aca))
                                .cursor_pointer()
                                .child(div().truncate().child(link.label))
                                .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                                    if let Some(from) = &from {
                                        this.follow_link(
                                            from.clone(),
                                            link.target.clone(),
                                            event.modifiers().secondary(),
                                            w,
                                            cx,
                                        );
                                    }
                                }))
                        }))
                    })
                    .when(self.ui.right_mode == 4, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .pb_3()
                                .child("属性")
                                .child(tool("new-property", "plus", "添加属性").on_click(
                                    cx.listener(|this, _, w, cx| this.edit_property("", "", w, cx)),
                                )),
                        )
                        .children(properties.iter().enumerate().map(|(i, p)| {
                            let name = p.name.clone();
                            let value = p.value.clone();
                            div()
                                .id(("property", i))
                                .p_2()
                                .cursor_pointer()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x999999))
                                        .child(name.clone()),
                                )
                                .child(
                                    div().truncate().child(
                                        serde_json::from_str::<String>(&value)
                                            .unwrap_or_else(|_| value.clone()),
                                    ),
                                )
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.edit_property(&name, &value, w, cx)
                                }))
                        }))
                    })
                    .when(self.ui.right_mode == 3, |s| {
                        s.child(self.tags_panel(window, cx))
                    }),
            )
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn navigate_tags(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.navigate_tags_modified(key, false, window, cx)
    }
    fn navigate_tags_modified(
        &mut self,
        key: &str,
        combine: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use inkstone::tags::{Navigation, rows};
        let action = inkstone::tags::navigate(
            &rows(&self.index, &self.ui.prefs.tags),
            self.ui.tags_selected.as_deref(),
            key,
        );
        match action {
            Navigation::None => return false,
            Navigation::Select(tag) => self.ui.tags_selected = Some(tag.to_lowercase()),
            Navigation::Fold(tag, collapsed) => {
                let key = tag.to_lowercase();
                self.ui.tags_selected = Some(key.clone());
                if collapsed {
                    self.ui.prefs.tags.collapsed.insert(key);
                } else {
                    self.ui.prefs.tags.collapsed.remove(&key);
                }
                self.persist_workspace(cx);
            }
            Navigation::Open(tag) => {
                self.search_tag(&tag, combine, window, cx);
            }
        }
        if let Some(i) = rows(&self.index, &self.ui.prefs.tags)
            .iter()
            .position(|row| Some(row.tag.to_lowercase()) == self.ui.tags_selected)
        {
            self.ui.tags_scroll.scroll_to_item(i);
        }
        cx.notify();
        true
    }
    pub(super) fn search_tag(
        &mut self,
        tag: &str,
        combine: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = if self.fulltext {
            self.search.read(cx).value().to_string()
        } else {
            self.ui.prefs.search_query.clone()
        };
        let query = inkstone::tags::search_query(&current, tag, combine);
        self.ui.prefs.search_query = query.clone();
        self.search
            .update(cx, |s, cx| s.set_value(query, window, cx));
        self.focus_search(true, window, cx);
    }
    fn tags_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        use inkstone::tags::{Sort, rows};
        let options = &self.ui.prefs.tags;
        let items = rows(&self.index, options);
        let sort = options.sort;
        let weak = cx.entity().downgrade();
        let sorting =
            tool("tags-sort", "arrow-up-down", "标签排序").dropdown_menu(move |mut menu, _, _| {
                for (value, label) in [
                    (Sort::Name, "标签名（升序）"),
                    (Sort::NameDescending, "标签名（降序）"),
                    (Sort::Frequency, "使用次数（从多到少）"),
                    (Sort::FrequencyAscending, "使用次数（从少到多）"),
                ] {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(sort == value).on_click(
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.ui.prefs.tags.sort = value;
                                this.persist_workspace(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
        div()
            .track_focus(&self.ui.tags_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                if this.ui.tags_focus.is_focused(w)
                    && this.navigate_tags_modified(
                        &event.keystroke.key,
                        event.keystroke.modifiers.control,
                        w,
                        cx,
                    )
                {
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pb_2()
                    .child(sorting)
                    .child(
                        tool("tags-hierarchy", "network", "显示嵌套标签")
                            .toggled(options.hierarchy)
                            .selected(options.hierarchy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.prefs.tags.hierarchy = !this.ui.prefs.tags.hierarchy;
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        tool("tags-collapse", "fold-vertical", "全部展开或折叠")
                            .disabled(!options.hierarchy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut expanded = this.ui.prefs.tags.clone();
                                expanded.collapsed.clear();
                                expanded.show_filter = false;
                                let parents: std::collections::BTreeSet<_> =
                                    rows(&this.index, &expanded)
                                        .into_iter()
                                        .filter(|row| row.children)
                                        .map(|row| row.tag.to_lowercase())
                                        .collect();
                                if parents.is_subset(&this.ui.prefs.tags.collapsed) {
                                    this.ui.prefs.tags.collapsed.clear();
                                } else {
                                    this.ui.prefs.tags.collapsed = parents;
                                }
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        tool("tags-filter", "search", "筛选标签")
                            .toggled(options.show_filter)
                            .selected(options.show_filter)
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.ui.prefs.tags.show_filter = !this.ui.prefs.tags.show_filter;
                                if this.ui.prefs.tags.show_filter {
                                    this.ui.tags_filter.update(cx, |s, cx| s.focus(w, cx));
                                } else {
                                    this.ui.prefs.tags.query.clear();
                                    this.ui
                                        .tags_filter
                                        .update(cx, |s, cx| s.set_value("", w, cx));
                                }
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(options.show_filter, |s| {
                s.child(
                    div()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                            if event.keystroke.key == "down" {
                                w.focus(&this.ui.tags_focus, cx);
                                this.navigate_tags("down", w, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(Input::new(&self.ui.tags_filter)),
                )
            })
            .when_some(
                if options.show_filter {
                    inkstone::search::Query::parse(&options.query).err()
                } else {
                    None
                },
                |s, error| s.child(div().text_sm().text_color(rgb(0xe87979)).child(error)),
            )
            .when(
                items.is_empty() && options.show_filter && !options.query.is_empty(),
                |s| {
                    s.child(div().text_sm().text_color(rgb(0x999999)).child(
                        if options.show_filter && !options.query.is_empty() {
                            "未找到匹配标签"
                        } else {
                            "没有标签"
                        },
                    ))
                },
            )
            .child(
                div()
                    .id("tag-rows")
                    .max_h((window.viewport_size().height - px(210.)).max(px(100.)))
                    .overflow_y_scroll()
                    .track_scroll(&self.ui.tags_scroll)
                    .children(items.into_iter().enumerate().map(|(i, row)| {
                        let tag = row.tag;
                        let fold_key = tag.to_lowercase();
                        let label = if options.hierarchy {
                            tag.rsplit('/').next().unwrap_or(&tag).to_string()
                        } else {
                            tag.clone()
                        };
                        div()
                            .id(("tag", i))
                            .when(
                                self.ui.tags_selected.as_deref() == Some(fold_key.as_str()),
                                |s| s.bg(rgba(0x88888833)),
                            )
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(27.))
                            .text_size(px(13.))
                            .rounded(px(4.))
                            .py_1()
                            .pl(px(row.depth as f32 * 17.))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0x88888822)))
                            .child(div().w(px(20.)).flex_shrink_0().when(row.children, |s| {
                                s.child(
                                    Button::new(("tag-fold", i))
                                        .ghost()
                                        .compact()
                                        .w(px(20.))
                                        .h(px(24.))
                                        .label(if row.collapsed { "›" } else { "⌄" })
                                        .accessibility_label(format!("展开或折叠标签 {tag}"))
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            cx.stop_propagation();
                                            this.ui.tags_selected = Some(fold_key.clone());
                                            w.focus(&this.ui.tags_focus, cx);
                                            if !this.ui.prefs.tags.collapsed.remove(&fold_key) {
                                                this.ui
                                                    .prefs
                                                    .tags
                                                    .collapsed
                                                    .insert(fold_key.clone());
                                            }
                                            this.persist_workspace(cx);
                                            cx.notify();
                                        })),
                                )
                            }))
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x999999))
                                    .child(row.count.to_string()),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                                this.search_tag(&tag, event.modifiers().control, w, cx);
                            }))
                    })),
            )
            .into_any_element()
    }
    fn editor_group(
        &self,
        index: Option<usize>,
        pane: Option<Entity<EditorPane>>,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = index
            .and_then(|i| self.tabs.get(i))
            .filter(|t| !t.path.as_os_str().is_empty());
        let pane = if active.is_some() { pane } else { None };
        let title = active
            .map(|t| {
                t.path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or("新标签页".into());
        let breadcrumb = active
            .map(|t| {
                t.path
                    .to_string_lossy()
                    .replace('\\', " / ")
                    .trim_end_matches(".md")
                    .to_string()
            })
            .unwrap_or_default();
        let reading = pane.as_ref().is_some_and(|p| p.read(cx).reading);
        let title_tab_id = active.map(|tab| tab.id);
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(self.bg())
            .when(self.ui.prefs.show_view_header, |s| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(40.))
                        .px_3()
                        .gap_1()
                        .child(self.navigation_button(pane.clone(), false, secondary, cx))
                        .child(self.navigation_button(pane.clone(), true, secondary, cx))
                        .child(
                            div()
                                .flex_1()
                                .text_center()
                                .text_size(px(13.))
                                .text_color(rgb(0x999999))
                                .child(breadcrumb),
                        )
                        .child(
                            tool(
                                "read-mode",
                                if reading { "pencil" } else { "book-open" },
                                "切换阅读视图 Ctrl+E",
                            )
                            .on_click(cx.listener(|this, _, w, cx| this.execute_command(6, w, cx))),
                        )
                        .child(
                            tool(
                                if secondary {
                                    "secondary-file-menu"
                                } else {
                                    "file-menu"
                                },
                                "ellipsis-vertical",
                                "更多选项",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.more = !this.ui.more;
                                cx.notify();
                            })),
                        ),
                )
            })
            .when(pane.is_some() && self.ui.prefs.show_inline_title, |s| {
                s.child(div().px(px(48.)).pt(px(12.)).child(
                    if let Some(edit) = self.ui.inline_title.as_ref().filter(|edit| {
                        active.is_some_and(|tab| tab.id == edit.id) && edit.secondary == secondary
                    }) {
                        div()
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if event.keystroke.key == "escape" {
                                    this.cancel_inline_title(window, cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .child(
                                Input::new(&edit.input)
                                    .readonly(self.ui.file_operation)
                                    .appearance(false)
                                    .bordered(false)
                                    .text_size(px(self.ui.prefs.font_size
                                        * inkstone::markdown::HEADING_SCALES[0]))
                                    .font_weight(FontWeight::BOLD)
                                    .line_height(relative(1.2))
                                    .h(px(self.ui.prefs.font_size
                                        * inkstone::markdown::HEADING_SCALES[0]
                                        * 1.2
                                        + 8.)),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .id(if secondary {
                                "secondary-inline-title"
                            } else {
                                "inline-title"
                            })
                            .cursor_text()
                            .text_size(px(
                                self.ui.prefs.font_size * inkstone::markdown::HEADING_SCALES[0]
                            ))
                            .line_height(relative(1.2))
                            .font_weight(FontWeight::BOLD)
                            .child(title)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(index) = this
                                    .tabs
                                    .iter()
                                    .position(|tab| Some(tab.id) == title_tab_id)
                                {
                                    this.begin_inline_title(index, secondary, window, cx);
                                }
                            }))
                            .into_any_element()
                    },
                ))
            })
            .child(div().flex_1().min_h_0().children(pane))
            .when(active.is_none(), |s| {
                s.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            Button::new("empty-new")
                                .ghost()
                                .label("创建新文件 (Ctrl + N)")
                                .on_click(cx.listener(|this, _, w, cx| this.focus_new(w, cx))),
                        )
                        .child(
                            Button::new("empty-open")
                                .ghost()
                                .label("打开文件 (Ctrl + O)")
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.focus_search(false, w, cx)),
                                ),
                        )
                        .child(
                            Button::new("empty-vault")
                                .ghost()
                                .label("打开笔记库")
                                .on_click(cx.listener(|this, _, w, cx| this.choose_vault(w, cx))),
                        ),
                )
            })
            .relative()
            .id(if secondary {
                "secondary-editor-group"
            } else {
                "primary-editor-group"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if secondary {
                        this.focus_secondary(cx);
                    } else if let Some(index) = index {
                        this.views.secondary_focused = false;
                        this.views.main = Some(this.tabs[index].id);
                        this.active = Some(index);
                        this.run_search(cx);
                        cx.notify();
                    }
                }),
            )
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.ui.prefs.auto_reveal_file && self.ui.prefs.left_open && self.ui.left_mode == 0 {
            let key = self
                .active
                .and_then(|i| self.tabs.get(i))
                .filter(|t| !t.path.as_os_str().is_empty())
                .map(|t| (self.generation, t.path.clone()));
            if key.is_none() {
                self.ui.last_revealed_file = None;
            } else if self.ui.last_revealed_file != key {
                cx.defer_in(_window, |this, _, cx| {
                    if this.ui.prefs.auto_reveal_file
                        && this.ui.prefs.left_open
                        && this.ui.left_mode == 0
                    {
                        this.reveal_current_file(cx);
                    }
                });
            }
        } else {
            self.ui.last_revealed_file = None;
        }
        let selected_tab = self.main_tab();
        let scroll_key = TabScrollKey {
            selected: selected_tab.and_then(|i| self.tabs.get(i)).map(|t| t.id),
            order: self.tabs.iter().map(|t| t.id).collect(),
            widths: [
                f32::from(_window.viewport_size().width).to_bits(),
                if self.ui.prefs.left_open {
                    self.ui.prefs.left_width.to_bits()
                } else {
                    0
                },
                if self.ui.prefs.right_open {
                    self.ui.prefs.right_width.to_bits()
                } else {
                    0
                },
            ],
        };
        if self.ui.tab_scroll_key.as_ref() != Some(&scroll_key) {
            self.ui.tab_scroll_key = Some(scroll_key);
            if let Some(i) = selected_tab {
                self.ui.tab_scroll.scroll_to_item(i);
            }
            cx.defer_in(_window, |this, _, cx| {
                if let Some(i) = this.main_tab() {
                    this.ui.tab_scroll.scroll_to_item(i);
                }
                cx.notify();
            });
        }
        if self.ui.link_update.is_some() && !self.ui.modal_focus.contains_focused(_window, cx) {
            _window.focus(&self.ui.modal_focus, cx);
        }
        let active = self.active.and_then(|i| self.tabs.get(i));
        let title = active
            .filter(|tab| !tab.path.as_os_str().is_empty())
            .map(|t| {
                t.path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or("新标签页".into());
        let pane = self.current_pane();
        let count = pane
            .as_ref()
            .filter(|_| !self.graph_open)
            .map(|p| {
                let counts = p.update(cx, |pane, cx| pane.text_counts(cx));
                format!("{} 个词  {} 个字符", counts.words, counts.characters)
            })
            .unwrap_or_default();
        let error = active.and_then(|t| t.save.error.borrow().clone());
        _window.set_window_title(&format!(
            "{} - 砚台",
            if self.graph_open {
                "关系图谱"
            } else {
                &title
            }
        ));
        let main_index = self.main_tab();
        let main_pane = main_index
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.pane.clone());
        let primary = self.editor_group(main_index, main_pane, false, cx);
        let center = if let Some(split) = &self.views.split {
            let secondary = self.editor_group(
                self.tabs.iter().position(|t| t.id == split.source),
                Some(split.pane.clone()),
                true,
                cx,
            );
            gpui_component::resizable::ResizablePanelGroup::new((
                "split-editors",
                usize::from(self.views.vertical),
            ))
            .axis(if self.views.vertical {
                Axis::Vertical
            } else {
                Axis::Horizontal
            })
            .child(
                resizable_panel()
                    .size_range(px(150.)..px(10000.))
                    .child(primary),
            )
            .child(
                resizable_panel()
                    .size_range(px(150.)..px(10000.))
                    .child(secondary),
            )
            .into_any_element()
        } else {
            primary
        };
        let center = if self.graph_open {
            div()
                .size_full()
                .children(self.graph.clone())
                .into_any_element()
        } else {
            center
        };
        let weak = cx.entity().downgrade();
        let left_open = self.ui.prefs.left_open;
        let right_open = self.ui.prefs.right_open;
        let panels = h_resizable((
            SharedString::from(format!(
                "workspace-panels-{}-{}-{}-{}",
                self.generation,
                self.ui.prefs.left_width.to_bits(),
                self.ui.prefs.right_width.to_bits(),
                f32::from(_window.viewport_size().width).to_bits()
            )),
            usize::from(left_open) + 2 * usize::from(right_open),
        ))
        .when(left_open, |s| {
            s.child(
                resizable_panel()
                    .size(px(self.ui.prefs.left_width))
                    .flex_none()
                    .size_range(px(180.)..px(500.))
                    .child(self.left_panel(cx)),
            )
        })
        .child(
            resizable_panel()
                .size_range(px(260.)..px(10000.))
                .child(center),
        )
        .when(right_open, |s| {
            s.child(
                resizable_panel()
                    .size(px(self.ui.prefs.right_width))
                    .flex_none()
                    .size_range(px(180.)..px(500.))
                    .child(self.right_panel(_window, cx)),
            )
        })
        .on_resize(move |state, _, cx| {
            let sizes = state.read(cx).sizes().clone();
            let _ = weak.update(cx, |this, cx| {
                if left_open && let Some(size) = sizes.first() {
                    this.ui.prefs.left_width = f32::from(*size);
                }
                if right_open && let Some(size) = sizes.last() {
                    this.ui.prefs.right_width = f32::from(*size);
                }
                this.persist_workspace(cx);
            });
        });
        div()
            .id("workspace")
            .track_focus(&self.ui.workspace_focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(self.bg())
            .text_color(self.fg())
            .text_size(px(14.))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, w, cx| {
                this.import_files(paths.paths().to_vec(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &NewTab, w, cx| this.new_blank(w, cx)))
            .on_action(cx.listener(|this, _: &SplitRight, w, cx| this.split_active(false, w, cx)))
            .on_action(cx.listener(|this, _: &SplitDown, w, cx| this.split_active(true, w, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleTaskLine, w, cx| this.execute_command(35, w, cx)),
            )
            .on_action(cx.listener(|this, _: &Save, w, cx| this.save_all(w, cx)))
            .on_action(cx.listener(|this, _: &OpenVault, w, cx| this.choose_vault(w, cx)))
            .on_action(cx.listener(|this, _: &QuickOpen, w, cx| this.focus_search(false, w, cx)))
            .on_action(cx.listener(|this, _: &FullSearch, w, cx| this.focus_search(true, w, cx)))
            .on_action(cx.listener(|this, _: &NewNote, w, cx| this.focus_new(w, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, w, cx| this.close_tab(w, cx)))
            .on_action(cx.listener(|this, _: &CommandPalette, w, cx| this.open_commands(w, cx)))
            .on_action(cx.listener(|this, _: &ClosePalette, w, cx| this.close_overlays(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleLeft, w, cx| this.execute_command(12, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleRight, w, cx| this.execute_command(13, w, cx)))
            .on_action(cx.listener(|this, _: &Settings, w, cx| this.execute_command(14, w, cx)))
            .on_action(cx.listener(|this, _: &NavigateBack, w, cx| this.navigate(false, w, cx)))
            .on_action(cx.listener(|this, _: &NavigateForward, w, cx| this.navigate(true, w, cx)))
            .on_action(cx.listener(|this, _: &NextTab, w, cx| this.cycle_tab(false, w, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, w, cx| this.cycle_tab(true, w, cx)))
            .on_action(cx.listener(|this, _: &ReopenTab, w, cx| this.execute_command(17, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleReading, w, cx| this.execute_command(6, w, cx)))
            .on_action(cx.listener(|this, _: &RenameNote, w, cx| this.execute_command(8, w, cx)))
            .on_action(cx.listener(|this, _: &Bold, w, cx| this.execute_command(24, w, cx)))
            .on_action(cx.listener(|this, _: &Italic, w, cx| this.execute_command(25, w, cx)))
            .on_action(cx.listener(|this, _: &InsertLink, w, cx| this.execute_command(59, w, cx)))
            .child(
                TitleBar::new()
                    .h(px(40.))
                    .pl_0()
                    .bg(self.side())
                    .when(self.ui.prefs.show_ribbon, |bar| {
                        bar.child(div().w(px(44.)).flex_shrink_0())
                    })
                    .when(left_open, |s| {
                        s.child(
                            div()
                                .w(px(self.ui.prefs.left_width))
                                .flex_shrink_0()
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(self.left_header(cx)),
                        )
                    })
                    .child(div().flex_1().min_w_0().child(self.tab_header(cx)))
                    .when(right_open, |s| {
                        s.child(
                            div()
                                .w(px((self.ui.prefs.right_width - 102.).max(148.)))
                                .flex_shrink_0()
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(self.right_header(cx)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(self.ui.prefs.show_ribbon, |body| {
                        body.child(self.ribbon(cx))
                    })
                    .child(div().flex_1().min_w_0().h_full().child(panels)),
            )
            .when(error.is_some(), |s| {
                s.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_color(rgb(0xe4a66a))
                        .children(error),
                )
            })
            .child(
                div()
                    .id("workspace-status-bar")
                    .debug_selector(|| "workspace-status-bar".into())
                    .occlude()
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .max_w(_window.viewport_size().width * 0.8)
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .bg(self.bg())
                    .text_size(px(12.))
                    .text_color(rgb(if self.ui.prefs.light {
                        0x5c5c5c
                    } else {
                        0x999999
                    }))
                    .when(!self.status.is_empty(), |bar| {
                        bar.child(div().truncate().child(self.status.clone()))
                    })
                    .child(count),
            )
            .when(
                self.command_open
                    || self.ui.quick_open
                    || self.ui.name_mode.is_some()
                    || self.ui.property_open
                    || self.ui.settings
                    || self.ui.trash_open
                    || self.ui.link_update.is_some(),
                |s| s.child(self.modal(_window, cx)),
            )
            .when(self.ui.more, |s| {
                s.child(
                    div()
                        .absolute()
                        .right(px(if right_open {
                            self.ui.prefs.right_width + 12.
                        } else {
                            12.
                        }))
                        .top(px(112.))
                        .w(px(250.))
                        .p_2()
                        .rounded(px(8.))
                        .bg(self.bg())
                        .border_1()
                        .border_color(self.border())
                        .shadow_lg()
                        .children([6, 7, 8, 15, 11, 18, 19, 10, 16].into_iter().map(|id| {
                            let (_, label, _) = COMMANDS[id];
                            div()
                                .id(("menu-item", id))
                                .p_2()
                                .cursor_pointer()
                                .rounded(px(4.))
                                .hover(|s| s.bg(rgba(0x88888822)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.execute_command(id, w, cx)
                                }))
                        })),
                )
            })
    }
}

impl Workspace {
    fn modal(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
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
            .flex()
            .flex_col()
            .w(px(if self.ui.settings { 900. } else { 580. }))
            .when(picker, |s| s.w(px(700.)))
            .max_w((window.viewport_size().width - px(32.)).max(px(280.)))
            .max_h(px(if self.ui.settings { 700. } else { 650. }).min(available_height))
            .when(self.ui.settings, |s| s.h(px(700.).min(available_height)))
            .p_3()
            .gap_2()
            .when(picker, |s| s.p_0().gap_0().overflow_hidden())
            .when(self.ui.settings, |s| s.p_0().gap_0().overflow_hidden())
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
                        s.h(px(32.))
                            .flex_shrink_0()
                            .px_3()
                            .border_b_1()
                            .border_color(self.border())
                    })
                    .child(if self.command_open {
                        "命令面板"
                    } else if self.ui.quick_open {
                        if self.ui.template_mode {
                            "插入模板"
                        } else {
                            "快速切换"
                        }
                    } else if self.ui.property_open {
                        "编辑属性"
                    } else if self.ui.link_update.is_some() {
                        "更新内部链接"
                    } else if self.ui.settings {
                        "设置"
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
                        .max_h(px(384.).min((available_height - px(80.)).max(px(40.))))
                        .children(self.filtered_commands(cx).into_iter().enumerate().map(
                            |(i, (id, label, _))| {
                                div()
                                    .id(("command", id))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(33.))
                                    .px_3()
                                    .gap_3()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(self.ui.selected == i, |s| s.bg(rgba(0x88888822)))
                                    .hover(|s| s.bg(rgba(0x88888822)))
                                    .child(div().flex_1().min_w_0().truncate().child(label))
                                    .child(
                                        div()
                                            .text_color(rgb(0x888888))
                                            .text_size(px(12.))
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
                    (self.visible_search_hits(true, cx).len().max(1) as f32 * 33. + 16.).min(360.);
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
                        .text_size(px(11.))
                        .text_color(rgb(0x999999))
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
                            .text_xs()
                            .text_color(rgb(0x999999))
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
                            inkstone::properties::Kind::Date | inkstone::properties::Kind::DateTime
                        ),
                        |s| {
                            let i = usize::from(
                                self.effective_property_kind(cx)
                                    == inkstone::properties::Kind::DateTime,
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
                            inkstone::properties::Kind::Checkbox | inkstone::properties::Kind::List
                        ),
                        |s| s.child(Input::new(&self.ui.property_value)),
                    )
                    .when(
                        self.effective_property_kind(cx) == inkstone::properties::Kind::List,
                        |s| s.child(self.property_list_control(cx)),
                    )
                    .when(!self.ui.property_error.is_empty(), |s| {
                        s.child(
                            div()
                                .text_sm()
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
            .when(self.ui.trash_open, |s| {
                s.child(
                    div()
                        .id("trash-items")
                        .max_h(px(450.))
                        .overflow_y_scroll()
                        .child(
                            div()
                                .p_2()
                                .text_color(rgb(0x999999))
                                .child("回收站 · 恢复到原目录"),
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
                                        .truncate()
                                        .child(e.original.to_string_lossy().to_string()),
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
                                .text_color(rgb(0x999999))
                                .child("未保存草稿 · 恢复为新笔记"),
                        )
                        .children(self.recoveries.iter().enumerate().map(|(i, e)| {
                            Button::new(("restore-draft", i))
                                .ghost()
                                .label(e.record.relative.to_string_lossy().to_string())
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.ui.trash_open = false;
                                    this.restore_draft(i, w, cx);
                                }))
                        })),
                )
            });
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
                    .child(div().text_sm().child(error))
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
        use inkstone::properties::Kind;
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
                    setting_switch("property-checkbox")
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
    fn settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        if matches!(self.ui.settings_tab, 5 | 6) {
            return self.appearance_settings_panel(self.ui.settings_tab == 6, cx);
        }
        if self.ui.settings_tab == 4 {
            return self.template_settings_panel(cx);
        }
        if self.ui.settings_tab == 3 {
            return self.daily_settings_panel(cx);
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
        let display = [
            EditorSetting::InlineTitle,
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
        behavior.push(self.settings_row(
            &format!("制表符宽度  {}", self.ui.prefs.tab_size),
            "设置制表符对应的空格数。",
            div().w(px(160.)).child(Slider::new(&self.ui.tab_width)),
            true,
            20.,
        ));
        let card = rgb(if self.ui.prefs.light {
            0xfafafa
        } else {
            0x212121
        });
        div()
            .flex()
            .gap_0()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                div()
                    .id("settings-content")
                    .track_scroll(&self.ui.settings_scroll)
                    .relative()
                    .vertical_scrollbar(&self.ui.settings_scroll)
                    .overflow_y_scroll()
                    .min_h_0()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .px(px(32.))
                    .py(px(48.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_5()
                            .flex_shrink_0()
                            .rounded(px(12.))
                            .bg(card)
                            .child(self.settings_row(
                                "新标签页的默认视图",
                                "选择新标签页使用编辑视图还是阅读视图。",
                                default_view,
                                false,
                                20.,
                            ))
                            .child(self.settings_row(
                                "默认编辑模式",
                                "选择编辑视图默认使用实时预览还是源码模式。",
                                editing_mode,
                                true,
                                20.,
                            )),
                    )
                    .child(self.settings_group("显示", display))
                    .child(self.settings_group("行为", behavior))
                    .child(
                        Button::new("settings-recovery")
                            .ghost()
                            .label("管理文件恢复")
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.ui.settings = false;
                                this.execute_command(16, w, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
