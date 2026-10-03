//! Native macOS menus. This module is excluded from other platform builds.
use crate::workspace::{self, Workspace};
use gpui::{App, Entity, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions};
use gpui_component::input;

actions!(
    app_menu,
    [Quit, Hide, HideOthers, ShowAll, Minimize, Zoom, Fullscreen]
);

pub fn init(workspace: &Entity<Workspace>, window: &Window, cx: &mut App) {
    let workspace = workspace.downgrade();
    let handle = window.window_handle();
    cx.on_action(move |_: &Quit, cx| {
        // Use the same save/conflict checks as closing the main window.
        if handle
            .update(cx, |_, window, cx| {
                let _ =
                    workspace.update(cx, |workspace, cx| workspace.request_app_quit(window, cx));
            })
            .is_err()
        {
            cx.quit();
        }
    });
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(move |_: &Minimize, cx| {
        let _ = handle.update(cx, |_, window, _| window.minimize_window());
    });
    cx.on_action(move |_: &Zoom, cx| {
        let _ = handle.update(cx, |_, window, _| window.zoom_window());
    });
    cx.on_action(move |_: &Fullscreen, cx| {
        let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", Fullscreen, None),
    ]);
    cx.set_menus([
        Menu::new("Inkstone").items([
            MenuItem::action("设置…", workspace::Settings),
            MenuItem::separator(),
            MenuItem::os_submenu("服务", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("隐藏 Inkstone", Hide),
            MenuItem::action("隐藏其他应用", HideOthers),
            MenuItem::action("显示全部", ShowAll),
            MenuItem::separator(),
            MenuItem::action("退出 Inkstone", Quit),
        ]),
        Menu::new("文件").items([
            MenuItem::action("打开笔记库…", workspace::OpenVault),
            MenuItem::action("快速打开…", workspace::QuickOpen),
            MenuItem::separator(),
            MenuItem::action("新建笔记…", workspace::NewNote),
            MenuItem::action("新建标签页", workspace::NewTab),
            MenuItem::action("重新打开关闭的标签页", workspace::ReopenTab),
            MenuItem::action("关闭标签页", workspace::CloseTab),
            MenuItem::separator(),
            MenuItem::action("保存全部", workspace::Save),
            MenuItem::action("重命名笔记…", workspace::RenameNote),
        ]),
        Menu::new("编辑").items([
            MenuItem::os_action("撤销", input::Undo, OsAction::Undo),
            MenuItem::os_action("重做", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("剪切", input::Cut, OsAction::Cut),
            MenuItem::os_action("复制", input::Copy, OsAction::Copy),
            MenuItem::os_action("粘贴", input::Paste, OsAction::Paste),
            MenuItem::os_action("全选", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("全文搜索…", workspace::FullSearch),
            MenuItem::separator(),
            MenuItem::action("加粗", workspace::Bold),
            MenuItem::action("斜体", workspace::Italic),
            MenuItem::action("插入链接", workspace::InsertLink),
        ]),
        Menu::new("视图").items([
            MenuItem::action("命令面板…", workspace::CommandPalette),
            MenuItem::action("切换阅读模式", workspace::ToggleReading),
            MenuItem::separator(),
            MenuItem::action("显示 / 隐藏左侧栏", workspace::ToggleLeft),
            MenuItem::action("显示 / 隐藏右侧栏", workspace::ToggleRight),
            MenuItem::separator(),
            MenuItem::action("向右拆分", workspace::SplitRight),
            MenuItem::action("向下拆分", workspace::SplitDown),
            MenuItem::separator(),
            MenuItem::action("切换全屏", Fullscreen),
        ]),
        Menu::new("导航").items([
            MenuItem::action("后退", workspace::NavigateBack),
            MenuItem::action("前进", workspace::NavigateForward),
            MenuItem::separator(),
            MenuItem::action("上一个标签页", workspace::PreviousTab),
            MenuItem::action("下一个标签页", workspace::NextTab),
        ]),
        Menu::new("窗口").items([
            MenuItem::action("最小化", Minimize),
            MenuItem::action("缩放", Zoom),
        ]),
    ]);
}
