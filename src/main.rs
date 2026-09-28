#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod editor;
mod editor_links;
mod workspace;
use gpui::{prelude::*, *};
use gpui_component::Root;
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            gpui_component::set_locale("zh-CN");
            cx.bind_keys([
                KeyBinding::new("ctrl-s", workspace::Save, None),
                KeyBinding::new("ctrl-o", workspace::OpenVault, None),
                KeyBinding::new("ctrl-p", workspace::QuickOpen, None),
                KeyBinding::new("ctrl-shift-f", workspace::FullSearch, None),
                KeyBinding::new("ctrl-shift-p", workspace::CommandPalette, None),
                KeyBinding::new("ctrl-n", workspace::NewNote, None),
                KeyBinding::new("ctrl-w", workspace::CloseTab, None),
                KeyBinding::new("escape", workspace::ClosePalette, None),
                KeyBinding::new(
                    "ctrl-enter",
                    gpui_component::input::GoToDefinition,
                    Some("Input"),
                ),
            ]);
            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(80.), px(60.)),
                            size(px(1200.), px(820.)),
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("砚台 / Inkstone".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| workspace::Workspace::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("无法创建 GPUI 窗口");
            })
            .detach();
        });
}
