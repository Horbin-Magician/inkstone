//! Native macOS menus. This module is excluded from other platform builds.
use crate::workspace::{self, Workspace};
use gpui::{App, Entity, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions};
use gpui_component::input;

actions!(
    app_menu,
    [Quit, Hide, HideOthers, ShowAll, Minimize, Zoom, Fullscreen]
);

pub fn init(workspace: &Entity<Workspace>, window: &Window, cx: &mut App) {
    let observed = workspace.clone();
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
            crate::background_sync::window_closed(cx);
        }
        // Each file window owns its save/conflict guard. Let every live window
        // handle Quit before the last-window callback ends the application.
        cx.propagate();
    });
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(move |_: &Minimize, cx| {
        if cx.active_window() != Some(handle) {
            cx.propagate();
            return;
        }
        let _ = handle.update(cx, |_, window, _| window.minimize_window());
    });
    cx.on_action(move |_: &Zoom, cx| {
        if cx.active_window() != Some(handle) {
            cx.propagate();
            return;
        }
        let _ = handle.update(cx, |_, window, _| window.zoom_window());
    });
    cx.on_action(move |_: &Fullscreen, cx| {
        if cx.active_window() != Some(handle) {
            cx.propagate();
            return;
        }
        let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", Fullscreen, None),
    ]);
    let mut previous = menu_bindings(|id| observed.read(cx).hotkeys(id));
    install_bindings(&previous, cx);
    cx.observe(&observed, move |workspace, cx| {
        let next = menu_bindings(|id| workspace.read(cx).hotkeys(id));
        if next.0 != previous.0 {
            install_bindings(&next, cx);
            previous = next;
        }
    })
    .detach();
}

type Bindings = (
    Vec<(usize, Vec<String>)>,
    Vec<&'static str>,
    Vec<KeyBinding>,
);

fn menu_bindings(mut hotkeys: impl FnMut(usize) -> Vec<String>) -> Bindings {
    let mut result: Bindings = (Vec::new(), Vec::new(), Vec::new());
    macro_rules! command {
        ($id:expr, $action:ident) => {{
            let keys = hotkeys($id);
            result.1.push(gpui::Action::name(&workspace::$action));
            for key in &keys {
                // Preferences are user-editable; malformed values must not panic.
                if gpui::Keystroke::parse(key).is_ok() {
                    result
                        .2
                        .push(KeyBinding::new(key, workspace::$action, None));
                }
            }
            result.0.push(($id, keys));
        }};
    }
    command!(14, Settings);
    command!(1, OpenVault);
    command!(2, QuickOpen);
    command!(0, NewNote);
    command!(34, NewTab);
    command!(17, ReopenTab);
    command!(5, CloseTab);
    command!(4, Save);
    command!(8, RenameNote);
    command!(3, FullSearch);
    command!(24, Bold);
    command!(25, Italic);
    command!(59, InsertLink);
    command!(39, CommandPalette);
    command!(6, ToggleReading);
    command!(12, ToggleLeft);
    command!(13, ToggleRight);
    command!(31, SplitRight);
    command!(32, SplitDown);
    command!(20, NavigateBack);
    command!(21, NavigateForward);
    command!(41, PreviousTab);
    command!(40, NextTab);
    result
}

fn install_bindings(bindings: &Bindings, cx: &mut App) {
    let retained: Vec<_> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| !bindings.1.contains(&binding.action().name()))
        .cloned()
        .collect();
    // GPUI has no per-action removal. Rebuild from an exact clone of every
    // unrelated binding, preserving component contexts, system keys and order.
    cx.clear_key_bindings();
    cx.bind_keys(retained);
    cx.bind_keys(bindings.2.clone());
    install_menus(cx);
}

fn install_menus(cx: &App) {
    cx.set_menus([
        Menu::new(crate::product::name()).items([
            MenuItem::action("设置…", workspace::Settings),
            MenuItem::separator(),
            MenuItem::os_submenu("服务", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(format!("隐藏 {}", crate::product::name()), Hide),
            MenuItem::action("隐藏其他应用", HideOthers),
            MenuItem::action("显示全部", ShowAll),
            MenuItem::separator(),
            MenuItem::action(format!("退出 {}", crate::product::name()), Quit),
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
            MenuItem::action("重命名笔记", workspace::RenameNote),
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Action, TestAppContext};

    #[gpui::test]
    fn menu_bindings_follow_overrides_clear_and_reset_without_accumulating(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-z", input::Undo, Some("Input")),
                KeyBinding::new("cmd-s", workspace::Save, None),
            ]);
        });
        for keys in [Some(vec![String::from("ctrl-alt-s")]), Some(vec![]), None] {
            let bindings = handle
                .update(cx, |w, _, _| {
                    menu_bindings(|id| {
                        if id == 4 {
                            keys.clone().unwrap_or_else(|| w.hotkeys(id))
                        } else {
                            w.hotkeys(id)
                        }
                    })
                })
                .unwrap();
            cx.update(|cx| {
                install_bindings(&bindings, cx);
                let count = cx.key_bindings().borrow().bindings().len();
                install_bindings(&bindings, cx);
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                assert_eq!(keymap.bindings().len(), count);
                let save: Vec<_> = keymap.bindings_for_action(&workspace::Save).collect();
                let expected = keys.clone().unwrap_or_else(|| vec!["cmd-s".into()]);
                assert_eq!(save.len(), expected.len());
                for (binding, key) in save.iter().zip(expected) {
                    assert_eq!(binding.keystrokes(), KeyBinding::new(&key, workspace::Save, None).keystrokes());
                }
                assert_eq!(keymap.bindings_for_action(&Quit).count(), 1);
                assert!(keymap.bindings().any(|b| b.action().name() == input::Undo.name() && b.predicate().is_some()));
            });
        }
    }
}
