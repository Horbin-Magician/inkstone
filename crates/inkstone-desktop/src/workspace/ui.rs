use super::commands::command;
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_component::slider::{SliderEvent, SliderState};
use gpui_component::{
    Disableable, Icon, TitleBar,
    button::*,
    resizable::{h_resizable, resizable_panel},
};
use inkstone_core::file_order::SortBy;
use inkstone_core::preferences::Preferences;

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
    pub view: inkstone_core::preferences::ViewState,
    pub history: inkstone_core::preferences::Navigation,
}
impl ClosedTab {
    pub fn new(
        view: inkstone_core::preferences::ViewState,
        mut history: inkstone_core::preferences::Navigation,
    ) -> Self {
        if history.entries.is_empty() {
            history.visit(view.path.clone());
        }
        history.record_at(history.cursor, view.clone());
        Self { view, history }
    }
}
#[cfg(test)]
impl From<&str> for ClosedTab {
    fn from(path: &str) -> Self {
        Self::new(
            inkstone_core::preferences::ViewState {
                path: path.into(),
                ..Default::default()
            },
            Default::default(),
        )
    }
}
#[cfg(test)]
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
    pub link_update: Option<LinkEdits>,
    pub link_update_scroll: UniformListScrollHandle,
    pub left_mode: usize,
    pub right_mode: usize,
    pub quick_open: bool,
    pub name_mode: Option<NameMode>,
    pub folder_target: Option<PathBuf>,
    pub tree_name: Option<super::tree_name::TreeName>,
    pub tree_active: Option<PathBuf>,
    pub settings: bool,
    pub settings_tab: usize,
    pub focus_mode: bool,
    pub settings_filter: Entity<InputState>,
    _settings_filter_subscription: Subscription,
    pub hotkey_recording: Option<usize>,
    pub hotkey_recording_focus: Option<FocusHandle>,
    pub hotkey_message: String,
    pub hotkey_filter: Entity<InputState>,
    _hotkey_subscription: Subscription,
    _hotkey_filter_subscription: Subscription,
    pub note_folder_input: Entity<InputState>,
    pub attachment_folder_input: Entity<InputState>,
    _location_subscriptions: Vec<Subscription>,
    pub property_open: bool,
    pub property_kind: inkstone_core::properties::Kind,
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
    pub outline_filter: Entity<InputState>,
    pub outline_filter_open: bool,
    pub outline: super::right_sidebar::OutlineState,
    _outline_filter_subscription: Subscription,
    pub tags_focus: FocusHandle,
    pub tags_selected: Option<String>,
    pub tags_scroll: ScrollHandle,
    pub search_collapsed: std::collections::BTreeSet<PathBuf>,
    pub search_group_scroll: ScrollHandle,
    pub search_group_query: String,
    pub search_limit: usize,
    pub search_has_more: bool,
    pub search_loading: bool,
    pub search_drafts_changed: bool,
    pub search_error: String,
    pub search_signature: Option<(String, bool, SortBy, bool)>,
    _tags_filter_subscription: Subscription,
    _property_list_subscription: Subscription,
    pub trash_open: bool,
    pub sync_recovery: super::sync_recovery::State,
    pub history: Option<super::recovery::Browser>,
    pub history_catalog: super::history_catalog::State,
    pub conflict_review: Option<super::conflicts::Review>,
    pub backup: super::backups::State,
    pub cloud_sync: super::cloud_sync::State,
    pub exporting: bool,
    pub attachment_manager: Option<super::attachments::Manager>,
    pub link_health: Option<super::link_health::Review>,
    pub bulk_edit: Option<super::bulk_edit::Review>,
    pub table_editor: Option<super::table_editor::Review>,
    pub bulk_preview_revision: u64,
    pub recovery_refresh: u64,
    pub trash: Vec<inkstone_core::vault::TrashEntry>,
    pub trash_metadata: std::collections::BTreeMap<PathBuf, inkstone_core::vault::TrashMetadata>,
    pub command: Entity<InputState>,
    pub selected: usize,
    pub modal_scroll: ScrollHandle,
    pub settings_scroll: ScrollHandle,
    pub recovery_scroll: ScrollHandle,
    pub hotkey_scroll: ScrollHandle,
    pub tab_scroll: ScrollHandle,
    pub tab_scroll_key: Option<TabScrollKey>,
    pub last_revealed_file: Option<(u64, PathBuf)>,
    pub middle_pressed_tab: Option<usize>,
    pub inline_title: Option<super::inline_title::InlineTitle>,
    pub modal_focus: FocusHandle,
    pub workspace_focus: FocusHandle,
    pub pending_command: Option<(PathBuf, usize)>,
    pub discard_workspace_on_close: bool,
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
        let outline_filter = cx.new(|cx| InputState::new(window, cx).placeholder("筛选大纲…"));
        let outline_filter_subscription =
            cx.subscribe(&outline_filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
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
        let settings_filter = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置…"));
        let settings_filter_subscription = cx.observe(&settings_filter, |_, _, cx| cx.notify());
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
                InputEvent::PressEnter { .. } if this.command_open => {
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
            link_update: None,
            link_update_scroll: UniformListScrollHandle::new(),
            left_mode: 0,
            right_mode: 0,
            quick_open: false,
            name_mode: None,
            tree_name: None,
            tree_active: None,
            folder_target: None,
            settings: false,
            settings_tab: 0,
            focus_mode: false,
            hotkey_recording: None,
            hotkey_recording_focus: None,
            hotkey_message: String::new(),
            hotkey_filter,
            settings_filter,
            _settings_filter_subscription: settings_filter_subscription,
            _hotkey_subscription: hotkey_subscription,
            _hotkey_filter_subscription: hotkey_filter_subscription,
            note_folder_input,
            attachment_folder_input,
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
            outline_filter,
            outline_filter_open: false,
            outline: Default::default(),
            _outline_filter_subscription: outline_filter_subscription,
            tags_focus: cx.focus_handle(),
            tags_selected: None,
            tags_scroll: ScrollHandle::new(),
            search_collapsed: Default::default(),
            search_group_scroll: ScrollHandle::new(),
            search_group_query: String::new(),
            search_limit: 200,
            search_loading: false,
            search_drafts_changed: false,
            search_has_more: false,
            search_error: String::new(),
            search_signature: None,
            _tags_filter_subscription: tags_filter_subscription,
            _property_list_subscription: property_list_subscription,
            trash_open: false,
            sync_recovery: Default::default(),
            history: None,
            history_catalog: Default::default(),
            conflict_review: None,
            backup: Default::default(),
            cloud_sync: super::cloud_sync::State::new(window, cx),
            exporting: false,
            attachment_manager: None,
            link_health: None,
            bulk_edit: None,
            table_editor: None,
            bulk_preview_revision: 0,
            recovery_refresh: 0,
            trash: vec![],
            trash_metadata: Default::default(),
            command,
            selected: 0,
            modal_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            recovery_scroll: ScrollHandle::new(),
            hotkey_scroll: ScrollHandle::new(),
            tab_scroll: ScrollHandle::new(),
            tab_scroll_key: None,
            last_revealed_file: None,
            middle_pressed_tab: None,
            inline_title: None,
            modal_focus: cx.focus_handle(),
            workspace_focus,
            pending_command: None,
            discard_workspace_on_close: false,
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
        let bg = crate::theme::palette(light).background.into();
        let side = crate::theme::palette(light).sidebar.into();
        let hover = crate::theme::palette(light).hover.into();
        let fg = crate::theme::palette(light).foreground.into();
        let muted = crate::theme::palette(light).muted.into();
        let border = crate::theme::palette(light).border.into();
        theme.font_size = px(14.);
        theme.radius = px(8.);
        theme.background = bg;
        theme.foreground = fg;
        theme.border = border;
        theme.input = side;
        theme.colors.list = side;
        theme.list_hover = hover;
        theme.list_active_border = crate::theme::palette(light).selected.into();
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
        theme.list_active = crate::theme::palette(light).selected.into();
        theme.tokens.list_hover =
            Hsla::from(rgba(if light { 0x00000008 } else { 0xffffff08 })).into();
        theme.primary = crate::theme::palette(light).accent.into();
        theme.primary_hover = rgb(if light { 0x1c6577 } else { 0x8dcedc }).into();
        theme.primary_active = rgb(if light { 0x175565 } else { 0x5bafc2 }).into();
        theme.primary_foreground = rgb(if light { 0xffffff } else { 0x142b33 }).into();
        theme.button_primary = theme.primary;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
        theme.button_primary_foreground = theme.primary_foreground;
        theme.ring = crate::theme::palette(light).accent.into();
        theme.switch_thumb = rgb(0xffffff).into();
        theme.slider_thumb = rgb(0xffffff).into();
        theme.selection = rgba(if light { 0x24778a33 } else { 0x72bfd040 }).into();
        theme.scrollbar_thumb = rgb(if light { 0xcccccc } else { 0x484848 }).into();
    });
}
impl Workspace {
    pub(super) fn empty_state(&self, symbol: &str, title: &str, description: &str) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .p_4()
            .text_center()
            .child(
                icon(symbol)
                    .size(px(24.))
                    .text_color(crate::theme::palette(self.ui.prefs.light).accent),
            )
            .child(
                div()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .text_size(px(MIN_UI_FONT_SIZE))
                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                    .child(description.to_string()),
            )
            .into_any_element()
    }
}

pub(super) fn icon(name: &str) -> Icon {
    if matches!(
        name,
        "panel-left" | "panel-left-filled" | "panel-right" | "panel-right-filled"
    ) {
        let divider = if name.starts_with("panel-left") {
            9
        } else {
            15
        };
        let fill = match name {
            "panel-left-filled" => {
                r#"<path d="M5 3h4v18H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z" fill="currentColor" stroke="none"/>"#
            }
            "panel-right-filled" => {
                r#"<path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4Z" fill="currentColor" stroke="none"/>"#
            }
            _ => "",
        };
        return Icon::default()
            .data(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{fill}<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M{divider} 3v18"/></svg>"#).as_bytes())
            .size(px(17.));
    }
    let shape = match name {
        "trash" => Some("M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"),
        "history" => Some("M3 3v6h6M3 9a9 9 0 1 1 0 6M12 7v5l3 2"),
        "code" => Some("m8 5-7 7 7 7m8-14 7 7-7 7"),
        "split-horizontal" => Some("M3 3h18v18H3zM12 3v18"),
        "split-vertical" => Some("M3 3h18v18H3zM3 12h18"),
        "text" => Some("M4 5h16M4 10h10M4 15h16M4 20h10"),
        "hash" => Some("M10 3 8 21M16 3l-2 18M4 9h17M3 15h17"),
        "check-square" => {
            Some("M9 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-8M9 11l3 3L22 4")
        }
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
pub(super) fn tool(id: &'static str, name: &str, tip: impl Into<SharedString>) -> Button {
    let tip = tip.into();
    Button::new(id)
        .accessibility_id(id)
        .accessibility_label(tip.clone())
        .ghost()
        .compact()
        .icon(icon(name))
        .tooltip(tip)
        .w(px(32.))
        .h(px(32.))
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
    pub(super) fn bg(&self) -> Rgba {
        crate::theme::palette(self.ui.prefs.light).background
    }
    pub(super) fn side(&self) -> Rgba {
        crate::theme::palette(self.ui.prefs.light).sidebar
    }
    fn fg(&self) -> Rgba {
        crate::theme::palette(self.ui.prefs.light).foreground
    }
    pub(super) fn border(&self) -> Rgba {
        crate::theme::palette(self.ui.prefs.light).border
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
        let panes: Vec<_> = self
            .tabs
            .iter()
            .map(|t| t.pane.clone())
            .chain(self.views.split.iter().map(|s| s.pane.clone()))
            .collect();
        for pane in panes {
            pane.update(cx, |pane, cx| {
                pane.font_size = p.font_size;
                pane.line_spacing = p.line_spacing;
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
    pub(super) fn prompt_name(
        &mut self,
        mut mode: NameMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == NameMode::Rename
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path.as_os_str().is_empty())
        {
            mode = NameMode::New;
        }
        let path = match mode {
            NameMode::Rename => self
                .active
                .and_then(|i| self.tabs.get(i))
                .map(|t| t.path.clone()),
            NameMode::RenameFolder => self.ui.folder_target.clone(),
            NameMode::New => {
                self.focus_new(window, cx);
                return;
            }
            NameMode::Folder => Some(
                self.ui
                    .tree_active
                    .clone()
                    .filter(|path| self.ui.folders.contains(path))
                    .unwrap_or_default(),
            ),
        };
        if let Some(path) = path {
            self.begin_tree_name(mode, path, window, cx);
        }
    }
    pub(super) fn submit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() || self.ui.link_update.is_some() {
            return;
        }
        if self.ui.tree_name.is_some() && !self.prepare_tree_name(window, cx) {
            return;
        }
        let Some(mode) = self.ui.name_mode.take() else {
            return;
        };
        match mode {
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
                let folder = PathBuf::from(self.name.read(cx).value().as_ref());
                let path = folder.clone();
                let generation = self.generation;
                let write_ticket = self.file_writes.begin();
                let task = cx
                    .background_executor()
                    .spawn(async move { vault.create_folder(&path) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        this.file_writes.finish(write_ticket);
                        if this.generation != generation {
                            return;
                        }
                        let message = match result {
                            Ok(()) => {
                                for path in folder.ancestors().filter(|p| !p.as_os_str().is_empty())
                                {
                                    if !this.ui.folders.iter().any(|p| p == path) {
                                        this.ui.folders.push(path.to_path_buf());
                                    }
                                }
                                this.ui.folders.sort();
                                this.folder_revision += 1;
                                this.ui.tree_folders = this.ui.folders.clone();
                                this.rebuild_sorted_tree(cx);
                                "文件夹已创建".into()
                            }
                            Err(e) => e.to_string(),
                        };
                        this.notifications.publish(message);
                        // Reconcile even on failure: mkdir may have created some parents.
                        this.structure_changed = true;
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
    pub(super) fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    fn select_view_mode(&mut self, mode: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |pane, cx| {
                pane.reading = mode == 0;
                if mode != 0 {
                    pane.live = mode == 1;
                }
                pane.focus_view(window, cx);
                cx.notify();
            });
            self.persist_workspace(cx);
            cx.notify();
        }
    }
    pub(super) fn refresh_trash(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        self.recoveries_loading = true;
        let request = self.ui.recovery_refresh;
        self.refresh_sync_recovery(cx);
        self.refresh_history_catalog(cx);
        self.refresh_trash_metadata(cx);
        let task = cx
            .background_executor()
            .spawn(async move { (vault.trash_entries(), vault.recovery_summaries()) });
        cx.spawn(async move |this, cx| {
            let (trash, recoveries) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                this.recoveries_loading = false;
                match trash {
                    Ok(entries) => this.ui.trash = entries,
                    Err(e) => this.notifications.publish(e.to_string()),
                }
                match recoveries {
                    Ok(entries) => this.recoveries = entries,
                    Err(e) => this.notifications.publish(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn restore_deleted(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(entry) = self.ui.trash.get(i).cloned() else {
            return;
        };
        let generation = self.generation;
        let write_ticket = self.file_writes.begin();
        let restored = entry.clone();
        let task = cx
            .background_executor()
            .spawn(async move { vault.restore_trash(&restored) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(write_ticket);
                if this.generation != generation {
                    return;
                }
                this.notifications.publish(match &result {
                    Ok(()) => "文件已恢复到原目录".into(),
                    Err(e) => e.to_string(),
                });
                if result.is_ok() {
                    if entry.directory {
                        this.structure_changed = true;
                    } else {
                        this.changed_paths.insert(entry.original);
                    }
                    this.schedule_auto_sync(true);
                }
                this.refresh_requested = true;
                this.refresh_trash(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.link_update.take().is_some() {
            self.notifications
                .publish("文件已移动，链接保持原样。".into());
            self.focus_after_link_update(window, cx);
            cx.notify();
            return;
        }
        self.command_open = false;
        self.close_quick_search(window, cx);
        self.cancel_tree_name(cx);
        self.ui.settings = false;
        self.ui.hotkey_recording = None;
        self.ui.hotkey_recording_focus = None;
        self.ui.property_open = false;
        self.ui.property_error.clear();
        self.ui.property_baseline = None;
        self.ui.property_original = None;
        self.ui.trash_open = false;
        self.ui.history = None;
        self.ui.history_catalog = Default::default();
        self.ui.attachment_manager = None;
        self.ui.link_health = None;
        self.ui.bulk_edit = None;
        self.ui.table_editor = None;
        self.ui.conflict_review = None;
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        if let Some(pane) = self.current_pane().filter(|_| {
            self.active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|tab| !tab.path.as_os_str().is_empty())
        }) {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        } else {
            window.focus(&self.ui.workspace_focus, cx);
        }
        cx.notify();
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
                    format!(
                        "{name}{}",
                        if tab.save.persistence.is_dirty() {
                            " •"
                        } else {
                            ""
                        }
                    ),
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
                            .rounded_t(px(9.))
                            .text_color(if selected {
                                self.fg()
                            } else {
                                crate::theme::palette(self.ui.prefs.light).muted
                            })
                            .cursor_pointer()
                            .when(!selected, |s| {
                                s.child(
                                    div()
                                        .absolute()
                                        .top(px(3.))
                                        .bottom(px(3.))
                                        .left(px(2.))
                                        .right(px(2.))
                                        .rounded(px(6.))
                                        .group_hover(format!("note-tab-{}", t.id), |style| {
                                            style.bg(rgba(0x88888818))
                                        }),
                                )
                            })
                            .when(selected, |s| {
                                s.bg(self.bg())
                                    .border_1()
                                    .border_b_0()
                                    .border_color(self.border())
                            })
                            .child(
                                div()
                                    .truncate()
                                    .flex_1()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .child(format!(
                                        "{}{}",
                                        if t.path.as_os_str().is_empty() {
                                            "新标签页".into()
                                        } else {
                                            t.path.file_stem().unwrap_or_default().to_string_lossy()
                                        },
                                        if t.save.persistence.has_conflict()
                                            || t.save.persistence.error().is_some()
                                        {
                                            " ⚠"
                                        } else if t.save.persistence.is_dirty() {
                                            " •"
                                        } else {
                                            ""
                                        }
                                    )),
                            )
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
                tool(
                    "add-tab",
                    "plus",
                    format!("新建标签页 {}", self.hotkey_label(34)),
                )
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
            .into_any_element()
    }
    fn editor_group(
        &self,
        index: Option<usize>,
        pane: Option<Entity<EditorPane>>,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        let active = index
            .and_then(|i| self.tabs.get(i))
            .filter(|t| !t.path.as_os_str().is_empty());
        let pane = if active.is_some() { pane } else { None };
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
        let menu_weak = cx.entity().downgrade();
        let menu_items: Vec<_> = [
            6, 7, 31, 32, 8, 15, 11, 23, 111, 99, 100, 103, 104, 19, 18, 10, 16,
        ]
        .into_iter()
        .filter_map(|id| command(id).map(|entry| (entry, self.hotkey_label(id))))
        .collect();
        let menu_tab_id = index.and_then(|i| self.tabs.get(i)).map(|tab| tab.id);
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(self.bg())
            .when(
                active.is_some() && self.ui.prefs.show_view_header && !self.ui.focus_mode,
                |s| {
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
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(colors.muted)
                                    .child(breadcrumb),
                            )
                            .child(
                                tool(
                                    "read-mode",
                                    if reading { "pencil" } else { "book-open" },
                                    format!("切换阅读视图 {}", self.hotkey_label(6)),
                                )
                                .when(active.is_none(), |s| s.hidden())
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.execute_command(6, w, cx)),
                                ),
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
                                .dropdown_menu_with_anchor(
                                    gpui::Anchor::TopRight,
                                    move |mut menu, _, _| {
                                        for (entry, shortcut) in &menu_items {
                                            let &(id, title, _) = *entry;
                                            if matches!(id, 31 | 8 | 23 | 10) {
                                                menu = menu.separator();
                                            }
                                            let label = if id == 6 {
                                                if reading {
                                                    "编辑视图"
                                                } else {
                                                    "阅读视图"
                                                }
                                            } else if id == 16 {
                                                "文件恢复"
                                            } else {
                                                title
                                            };
                                            let symbol = match id {
                                                6 => {
                                                    if reading {
                                                        "pencil"
                                                    } else {
                                                        "book-open"
                                                    }
                                                }
                                                7 => "code",
                                                31 => "split-horizontal",
                                                32 => "split-vertical",
                                                8 => "pencil",
                                                15 => "bookmark",
                                                11 | 19 => "copy",
                                                23 => "search",
                                                18 => "folder",
                                                10 => "trash",
                                                _ => "history",
                                            };
                                            let shortcut = shortcut.clone();
                                            let weak = menu_weak.clone();
                                            menu = menu.item(
                                                PopupMenuItem::element_with_label(
                                                    label,
                                                    move |_, _| {
                                                        div()
                                                            .w(px(220.))
                                                            .flex()
                                                            .items_center()
                                                            .gap_3()
                                                            .text_size(px(MIN_UI_FONT_SIZE))
                                                            .when(id == 10, |s| {
                                                                s.text_color(rgb(0xe76575))
                                                            })
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .truncate()
                                                                    .child(label),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(px(MIN_UI_FONT_SIZE))
                                                                    .text_color(colors.muted)
                                                                    .child(shortcut.clone()),
                                                            )
                                                    },
                                                )
                                                .icon(icon(symbol).size(px(16.)))
                                                .on_click(move |_, window, cx| {
                                                    let _ = weak.update(cx, |this, cx| {
                                                        if secondary {
                                                            this.focus_secondary(cx);
                                                        } else if let Some(i) =
                                                            this.tabs.iter().position(|tab| {
                                                                Some(tab.id) == menu_tab_id
                                                            })
                                                        {
                                                            this.focus_primary(i, window, cx);
                                                        }
                                                        this.execute_command(id, window, cx);
                                                    });
                                                }),
                                            );
                                        }
                                        menu
                                    },
                                ),
                            ),
                    )
                },
            )
            .when(active.is_some(), |s| {
                s.child(div().flex_1().min_h_0().children(pane))
            })
            .when(active.is_none(), |s| s.child(self.blank_note(cx)))
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
        if self.ui.tree_name.is_none()
            && self.ui.prefs.auto_reveal_file
            && self.ui.prefs.left_open
            && self.ui.left_mode == 0
        {
            let key = self
                .active
                .and_then(|i| self.tabs.get(i))
                .filter(|t| !t.path.as_os_str().is_empty())
                .map(|t| (self.generation, t.path.clone()));
            if key.is_none() {
                self.ui.last_revealed_file = None;
            } else if self.ui.last_revealed_file != key {
                cx.defer_in(_window, |this, _, cx| {
                    if this.ui.tree_name.is_none()
                        && this.ui.prefs.auto_reveal_file
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
        let has_workspace =
            self.vault.is_some() || self.tabs.iter().any(|tab| !tab.path.as_os_str().is_empty());
        let active = self.active.and_then(|i| self.tabs.get(i));
        let pane = self.current_pane();
        let status_mode = pane
            .as_ref()
            .filter(|_| active.is_some_and(|tab| !tab.path.as_os_str().is_empty()))
            .map(|pane| {
                let pane = pane.read(cx);
                if pane.reading {
                    ("book-open", "阅读视图")
                } else if pane.live {
                    ("pencil", "实时预览")
                } else {
                    ("code", "源码模式")
                }
            });
        let count = pane
            .as_ref()
            .filter(|_| active.is_some_and(|tab| !tab.path.as_os_str().is_empty()))
            .map(|p| {
                let counts = p.update(cx, |pane, cx| pane.text_counts(cx));
                format!("{} 个词  {} 个字符", counts.words, counts.characters)
            })
            .unwrap_or_default();
        let error = active.and_then(|t| t.save.persistence.error().clone());
        let save_status = active.filter(|t| !t.path.as_os_str().is_empty()).map(|t| {
            let label = if t.save.persistence.has_conflict() {
                "外部修改冲突"
            } else if t.save.persistence.error().is_some() {
                "保存失败"
            } else if t.save.persistence.is_saving() {
                "正在保存…"
            } else if t.save.persistence.is_dirty() {
                "尚未保存"
            } else {
                "已保存"
            };
            let detail = t.save.persistence.error().clone().unwrap_or_else(|| {
                format!(
                    "{}：{label}。{}",
                    t.path.display(),
                    if t.save.persistence.has_conflict() {
                        "点击比较并处理外部修改。"
                    } else {
                        "点击保存当前更改。"
                    }
                )
            });
            (label, detail, t.save.persistence.has_conflict())
        });
        _window.set_window_title(crate::product::name());
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
        let center = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .when(!self.ui.focus_mode, |s| s.child(self.tab_header(cx)))
            .child(div().flex_1().min_h_0().child(center));
        let weak = cx.entity().downgrade();
        let left_open = self.ui.prefs.left_open && !self.ui.focus_mode;
        let right_open = self.ui.prefs.right_open && !self.ui.focus_mode;
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
            .tab_group()
            .on_key_down(cx.listener(|this, event, window, cx| {
                if this.modal_is_open() {
                    this.navigate_modal_tab(event, window, cx);
                    return;
                }
                let key = &event.keystroke;
                if key.key == "tab"
                    && !key.modifiers.control
                    && !key.modifiers.alt
                    && !key.modifiers.platform
                {
                    if key.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            }))
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
            .on_action(
                cx.listener(|this, _: &gpui_component::input::Escape, w, cx| {
                    if this.ui.tree_name.is_some() {
                        this.cancel_tree_name(cx);
                        this.ui.workspace_focus.focus(w, cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
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
                    .h(crate::TITLE_BAR_HEIGHT)
                    .bg(self.side())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(crate::product::name()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            // Keep title-bar actions out of the native window drag hitbox.
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .when(has_workspace, |bar| {
                                bar.child(
                                    Button::new("toggle-focus-mode")
                                        .ghost()
                                        .compact()
                                        .label(if self.ui.focus_mode {
                                            "退出专注"
                                        } else {
                                            "专注"
                                        })
                                        .accessibility_label(if self.ui.focus_mode {
                                            "退出专注模式"
                                        } else {
                                            "进入专注模式"
                                        })
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.execute_command(109, w, cx)
                                        })),
                                )
                                .child(
                                    tool(
                                        "title-toggle-left",
                                        if left_open {
                                            "panel-left-filled"
                                        } else {
                                            "panel-left"
                                        },
                                        if left_open {
                                            "收起左侧栏"
                                        } else {
                                            "展开左侧栏"
                                        },
                                    )
                                    .on_click(
                                        cx.listener(|this, _, w, cx| {
                                            this.execute_command(12, w, cx)
                                        }),
                                    ),
                                )
                                .child(
                                    tool(
                                        "title-toggle-right",
                                        if right_open {
                                            "panel-right-filled"
                                        } else {
                                            "panel-right"
                                        },
                                        if right_open {
                                            "收起右侧栏"
                                        } else {
                                            "展开右侧栏"
                                        },
                                    )
                                    .on_click(
                                        cx.listener(|this, _, w, cx| {
                                            this.execute_command(13, w, cx)
                                        }),
                                    ),
                                )
                            })
                            .child(tool("title-settings", "settings", "设置").on_click(
                                cx.listener(|this, _, w, cx| this.execute_command(14, w, cx)),
                            )),
                    ),
            )
            .when(!has_workspace, |s| {
                if self.startup_pending || self.loading {
                    s.child(self.loading_workspace())
                } else {
                    s.child(self.welcome(cx))
                }
            })
            .when(has_workspace, |s| {
                s.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(div().flex_1().min_w_0().h_full().child(panels)),
                )
            })
            .when(error.is_some(), |s| {
                s.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_color(rgb(0xe4a66a))
                        .children(error)
                        .child(
                            Button::new("review-save-conflict")
                                .compact()
                                .label("比较并处理")
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.open_conflict_review(w, cx)),
                                ),
                        ),
                )
            })
            .when(has_workspace, |s| {
                s.child(
                    div()
                        .id("workspace-status-bar")
                        .debug_selector(|| "workspace-status-bar".into())
                        .occlude()
                        .w_full()
                        .flex_shrink_0()
                        .justify_end()
                        .border_t_1()
                        .border_color(self.border())
                        .flex()
                        .items_center()
                        .h(px(28.))
                        .px_2()
                        .gap_2()
                        .bg(self.bg())
                        .text_size(px(MIN_UI_FONT_SIZE))
                        .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                        .when(self.settings_save.error().is_some(), |bar| {
                            bar.child(
                                Button::new("retry-workspace-save")
                                    .ghost()
                                    .compact()
                                    .label("重试保存设置")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_save.retry();
                                        this.ui.discard_workspace_on_close = false;
                                        this.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("close-without-workspace")
                                    .ghost()
                                    .compact()
                                    .label("不保存布局并关闭")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.ui.discard_workspace_on_close = true;
                                        this.ui.window_close_requested = true;
                                        this.finish_window_close(window, cx);
                                    })),
                            )
                        })
                        .when(!self.index.errors.is_empty(), |bar| {
                            let detail = self
                                .index
                                .errors
                                .iter()
                                .take(20)
                                .map(|(path, error)| format!("{}：{error}", path.display()))
                                .collect::<Vec<_>>()
                                .join("\n");
                            bar.child(
                                Button::new("index-read-errors")
                                    .ghost()
                                    .compact()
                                    .label(format!("{} 个文件无法索引", self.index.errors.len()))
                                    .tooltip(detail),
                            )
                        })
                        .when(!self.notifications.text().is_empty(), |bar| {
                            bar.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(self.notifications.text().to_owned()),
                            )
                        })
                        .when_some(status_mode, |bar, (symbol, mode)| {
                            let weak = cx.entity().downgrade();
                            bar.child(
                                Button::new("status-backlinks")
                                    .ghost()
                                    .compact()
                                    .h(px(22.))
                                    .accessibility_label("显示反向链接")
                                    .child(
                                        div()
                                            .text_size(px(MIN_UI_FONT_SIZE))
                                            .text_color(
                                                crate::theme::palette(self.ui.prefs.light).muted,
                                            )
                                            .child(format!("{} 条反向链接", self.backlinks.len())),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.ui.focus_mode = false;
                                        this.ui.prefs.right_open = true;
                                        this.ui.right_mode = 1;
                                        this.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("status-editor-mode")
                                    .ghost()
                                    .compact()
                                    .w(px(24.))
                                    .h(px(22.))
                                    .icon(icon(symbol).size(px(15.)))
                                    .accessibility_label(format!("{mode}；选择视图模式"))
                                    .tooltip(mode)
                                    .dropdown_menu_with_anchor(
                                        gpui::Anchor::BottomRight,
                                        move |mut menu, _, _| {
                                            menu = menu.check_side(gpui_component::Side::Right);
                                            for (value, label, symbol) in [
                                                (0, "阅读视图", "book-open"),
                                                (2, "源码模式", "code"),
                                                (1, "实时预览", "pencil"),
                                            ] {
                                                let weak = weak.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(label)
                                                        .icon(icon(symbol))
                                                        .checked(mode == label)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = weak.update(cx, |this, cx| {
                                                                this.select_view_mode(
                                                                    value, window, cx,
                                                                )
                                                            });
                                                        }),
                                                );
                                            }
                                            menu
                                        },
                                    ),
                            )
                        })
                        .when_some(save_status, |bar, (label, detail, conflict)| {
                            bar.child(
                                Button::new("current-save-status")
                                    .ghost()
                                    .compact()
                                    .label(label)
                                    .accessibility_label(format!("当前笔记：{label}"))
                                    .tooltip(detail)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        if conflict {
                                            this.execute_command(100, w, cx);
                                        } else {
                                            this.save_all(w, cx);
                                        }
                                    })),
                            )
                        })
                        .child(div().px_1().child(count)),
                )
            })
            .when(self.modal_is_open(), |s| s.child(self.modal(_window, cx)))
    }
}
