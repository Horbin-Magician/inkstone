#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod editor;
mod editor_links;
#[cfg(feature = "metrics")]
mod metrics;
mod theme;
mod workspace;
use gpui::{prelude::*, *};
use gpui_component::Root;
fn main() {
    #[cfg(feature = "metrics")]
    metrics::init();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            gpui_component::set_locale("zh-CN");
            cx.bind_keys([
                KeyBinding::new("ctrl-s", workspace::Save, None),
                KeyBinding::new("ctrl-shift-o", workspace::OpenVault, None),
                KeyBinding::new("ctrl-o", workspace::QuickOpen, None),
                KeyBinding::new("ctrl-p", workspace::CommandPalette, None),
                KeyBinding::new("ctrl-shift-f", workspace::FullSearch, None),
                KeyBinding::new("ctrl-shift-p", workspace::CommandPalette, None),
                KeyBinding::new("ctrl-n", workspace::NewNote, None),
                KeyBinding::new("ctrl-\\", workspace::SplitRight, None),
                KeyBinding::new("ctrl-t", workspace::NewTab, None),
                KeyBinding::new("ctrl-w", workspace::CloseTab, None),
                KeyBinding::new("ctrl-shift-r", workspace::ToggleRight, None),
                KeyBinding::new("ctrl-,", workspace::Settings, None),
                KeyBinding::new("alt-left", workspace::NavigateBack, None),
                KeyBinding::new("alt-right", workspace::NavigateForward, None),
                KeyBinding::new("ctrl-tab", workspace::NextTab, None),
                KeyBinding::new("ctrl-shift-tab", workspace::PreviousTab, None),
                KeyBinding::new("ctrl-shift-t", workspace::ReopenTab, None),
                KeyBinding::new("ctrl-e", workspace::ToggleReading, None),
                KeyBinding::new("f2", workspace::RenameNote, None),
                KeyBinding::new("ctrl-b", workspace::Bold, None),
                KeyBinding::new("ctrl-i", workspace::Italic, None),
                KeyBinding::new("ctrl-k", workspace::InsertLink, None),
                KeyBinding::new("ctrl-z", gpui_component::input::Undo, None),
                KeyBinding::new("ctrl-y", gpui_component::input::Redo, None),
                KeyBinding::new("ctrl-l", workspace::ToggleTaskLine, None),
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
                        window_min_size: Some(size(px(800.), px(500.))),
                        ..gpui_component::TitleBar::window_options()
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
