#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
mod app_icon;
#[cfg(target_os = "macos")]
mod app_menu;
mod background_sync;
mod editor;
mod editor_links;
mod native_graphics;
mod product;
mod shortcuts;
#[cfg(test)]
mod test_support;
mod theme;
mod workspace;
use gpui::{prelude::*, *};
use gpui_component::Root;

const TITLE_BAR_HEIGHT: Pixels = px(40.);

fn main() {
    inkstone_core::startup_trace::mark("main");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx| {
            inkstone_core::startup_trace::mark("application_ready");
            gpui_kit::init(cx);
            #[cfg(target_os = "macos")]
            app_icon::init();
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            gpui_component::set_locale("zh-CN");
            cx.bind_keys([
                KeyBinding::new(&shortcuts::command_default("ctrl-s"), workspace::Save, None),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-o"),
                    workspace::OpenVault,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-o"),
                    workspace::QuickOpen,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-p"),
                    workspace::CommandPalette,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-f"),
                    workspace::FullSearch,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-p"),
                    workspace::CommandPalette,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-n"),
                    workspace::NewNote,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-\\"),
                    workspace::SplitRight,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-t"),
                    workspace::NewTab,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-w"),
                    workspace::CloseTab,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-r"),
                    workspace::ToggleRight,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-,"),
                    workspace::Settings,
                    None,
                ),
                KeyBinding::new("alt-left", workspace::NavigateBack, None),
                KeyBinding::new("alt-right", workspace::NavigateForward, None),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-tab"),
                    workspace::NextTab,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-tab"),
                    workspace::PreviousTab,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-shift-t"),
                    workspace::ReopenTab,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-e"),
                    workspace::ToggleReading,
                    None,
                ),
                KeyBinding::new("f2", workspace::RenameNote, None),
                KeyBinding::new(&shortcuts::command_default("ctrl-b"), workspace::Bold, None),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-i"),
                    workspace::Italic,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-k"),
                    workspace::InsertLink,
                    None,
                ),
                KeyBinding::new(
                    if cfg!(target_os = "macos") {
                        "cmd-z"
                    } else {
                        "ctrl-z"
                    },
                    gpui_component::input::Undo,
                    None,
                ),
                KeyBinding::new(
                    if cfg!(target_os = "macos") {
                        "cmd-shift-z"
                    } else {
                        "ctrl-y"
                    },
                    gpui_component::input::Redo,
                    None,
                ),
                KeyBinding::new(
                    &shortcuts::command_default("ctrl-l"),
                    workspace::ToggleTaskLine,
                    None,
                ),
                KeyBinding::new("escape", workspace::ClosePalette, None),
                KeyBinding::new(
                    "ctrl-enter",
                    gpui_component::input::GoToDefinition,
                    Some("Input"),
                ),
            ]);
            inkstone_core::startup_trace::mark("ui_framework_ready");
            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(80.), px(60.)),
                            // Opt-in native acceptance at the same minimum used below.
                            // Exact matching keeps inherited/empty values from changing startup.
                            if std::env::var("INKSTONE_ACCEPTANCE_MIN_WINDOW").as_deref() == Ok("1")
                            {
                                size(px(800.), px(500.))
                            } else {
                                size(px(1200.), px(820.))
                            },
                        ))),
                        window_min_size: Some(size(px(800.), px(500.))),
                        titlebar: Some(TitlebarOptions {
                            // Center AppKit's 14-point buttons in our custom title bar.
                            #[cfg(target_os = "macos")]
                            traffic_light_position: Some(point(
                                px(12.),
                                (TITLE_BAR_HEIGHT - px(14.)) / 2.,
                            )),
                            ..gpui_component::TitleBar::title_bar_options()
                        }),
                        ..gpui_component::TitleBar::window_options()
                    },
                    |window, cx| {
                        let main_window_id = window.window_handle().window_id();
                        cx.on_window_closed(move |cx, window_id| {
                            if window_id == main_window_id {
                                background_sync::window_closed(cx);
                            }
                        })
                        .detach();
                        inkstone_core::startup_trace::mark("window_created");
                        let view = cx.new(|cx| workspace::Workspace::new(window, cx));
                        inkstone_core::startup_trace::mark("workspace_created");
                        #[cfg(target_os = "macos")]
                        app_menu::init(&view, window, cx);
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("无法创建 GPUI 窗口");
            })
            .detach();
        });
}
