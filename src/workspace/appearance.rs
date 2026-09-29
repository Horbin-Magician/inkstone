use super::*;
use gpui_component::{button::Button, slider::Slider, switch::Switch};

impl Workspace {
    pub(super) fn appearance_settings_panel(
        &self,
        interface: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Button::new("theme-setting")
            .label(if self.ui.prefs.light {
                "浅色"
            } else {
                "深色"
            })
            .on_click(cx.listener(|this, _, w, cx| this.execute_command(22, w, cx)));
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
