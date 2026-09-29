use super::*;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::select::{SearchableVec, Select, SelectItem};
use gpui_component::{button::Button, slider::Slider, switch::Switch};
use inkstone::preferences::ThemeMode;

#[derive(Clone)]
pub(super) struct FontChoice {
    pub name: String,
    pub missing: bool,
}
impl SelectItem for FontChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        if self.name.is_empty() {
            "默认".into()
        } else if self.missing {
            format!("{}（未安装）", self.name).into()
        } else {
            self.name.clone().into()
        }
    }
    fn value(&self) -> &String {
        &self.name
    }
    fn disabled(&self) -> bool {
        self.missing
    }
}

impl Workspace {
    pub(super) fn sync_font_selects(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (role, value) in [
            &self.ui.prefs.interface_font,
            &self.ui.prefs.text_font,
            &self.ui.prefs.monospace_font,
        ]
        .into_iter()
        .enumerate()
        {
            let select = &self.ui.font_selects[role];
            if select.read(cx).selected_value() == Some(value) {
                continue;
            }
            let mut fonts: Vec<_> = std::iter::once(String::new())
                .chain(self.ui.available_fonts.iter().cloned())
                .map(|name| FontChoice {
                    name,
                    missing: false,
                })
                .collect();
            if !fonts.iter().any(|font| &font.name == value) {
                fonts.push(FontChoice {
                    name: value.clone(),
                    missing: !self
                        .ui
                        .available_fonts
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(value)),
                });
            }
            select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(fonts), window, cx);
                state.set_selected_value(value, window, cx);
            });
        }
    }
    pub(super) fn resolved_font(&self, requested: &str, fallback: &str) -> SharedString {
        self.ui
            .available_fonts
            .iter()
            .find(|font| font.eq_ignore_ascii_case(requested))
            .map_or_else(|| fallback.to_string(), Clone::clone)
            .into()
    }
    pub(super) fn apply_font_preferences(&self, cx: &mut App) {
        let interface = self.resolved_font(&self.ui.prefs.interface_font, &self.ui.default_fonts.0);
        let mono = self.resolved_font(&self.ui.prefs.monospace_font, &self.ui.default_fonts.1);
        let current = gpui_component::Theme::global(cx);
        if current.font_family == interface && current.mono_font_family == mono {
            return;
        }
        gpui_component::Theme::update(cx, |theme| {
            theme.font_family = interface;
            theme.mono_font_family = mono;
        });
    }
    pub(super) fn system_light(window: &Window) -> bool {
        matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        )
    }
    pub(super) fn set_theme(
        &mut self,
        theme: ThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.prefs.theme = theme;
        self.ui.prefs.light = theme.is_light(Self::system_light(window));
        ui::apply_theme(self.ui.prefs.light, cx);
        self.apply_editor_preferences(window, cx);
    }
    pub(super) fn update_system_theme(
        &mut self,
        light: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.prefs.theme != ThemeMode::System || self.ui.prefs.light == light {
            return;
        }
        self.ui.prefs.light = light;
        ui::apply_theme(light, cx);
        self.apply_editor_preferences(window, cx);
    }
    pub(super) fn appearance_settings_panel(
        &self,
        interface: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.ui.prefs.theme;
        let weak = cx.entity().downgrade();
        let theme = Button::new("theme-setting")
            .label(match selected {
                ThemeMode::System => "跟随系统",
                ThemeMode::Light => "浅色",
                ThemeMode::Dark => "深色",
            })
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in [
                    (ThemeMode::System, "跟随系统"),
                    (ThemeMode::Light, "浅色"),
                    (ThemeMode::Dark, "深色"),
                ] {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(value == selected)
                            .on_click(move |_, window, cx| {
                                let _ =
                                    weak.update(cx, |this, cx| this.set_theme(value, window, cx));
                            }),
                    );
                }
                menu
            });
        div()
            .flex()
            .gap_4()
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
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if interface { "界面" } else { "外观" }),
                    )
                    .when(!interface, |s| {
                        s.child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child("基础主题")
                                .child(theme),
                        )
                        .children(
                            ["界面字体", "正文字体", "等宽字体"]
                                .into_iter()
                                .enumerate()
                                .map(|(role, label)| {
                                    div()
                                        .flex()
                                        .justify_between()
                                        .items_center()
                                        .child(label)
                                        .child(
                                            div()
                                                .w(px(240.))
                                                .flex_shrink_0()
                                                .debug_selector(move || match role {
                                                    0 => "font-interface-trigger".into(),
                                                    1 => "font-text-trigger".into(),
                                                    _ => "font-monospace-trigger".into(),
                                                })
                                                .child(
                                                    Select::new(&self.ui.font_selects[role])
                                                        .w_full()
                                                        .accessibility_label(label)
                                                        .search_placeholder("搜索字体…")
                                                        .empty(|_, _| {
                                                            div().p_3().child("未找到字体")
                                                        }),
                                                ),
                                        )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(format!("正文字号  {}", self.ui.prefs.font_size))
                                .child(
                                    div()
                                        .w(px(160.))
                                        .child(Slider::new(&self.ui.font_size_slider)),
                                ),
                        )
                    })
                    .when(interface, |s| {
                        s.child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child("显示标签页标题栏")
                                .child(
                                    Switch::new("view-header-setting")
                                        .accessibility_label("显示标签页标题栏")
                                        .checked(self.ui.prefs.show_view_header)
                                        .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                            this.ui.prefs.show_view_header = *enabled;
                                            this.ui.more = false;
                                            this.persist_workspace(cx);
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}
