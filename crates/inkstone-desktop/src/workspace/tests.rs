use super::*;
use crate::test_support::PlatformKeys;
use core::prelude::v1::test;
#[gpui::test]
fn single_line_inputs_keep_text_and_caret_inside_the_frame(cx: &mut TestAppContext) {
    use gpui_component::{Sizable, Size};

    struct Probe {
        state: Entity<InputState>,
        size: Size,
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(220.)).child(
                Input::new(&self.state)
                    .with_size(self.size)
                    .prefix(div().w(px(14.)).h(px(14.)))
                    .suffix(div().w(px(32.)).h(px(20.)))
                    .cleanable(true),
            )
        }
    }

    cx.update(gpui_kit::init);
    let (probe, visual) = cx.add_window_view(|window, cx| Probe {
        state: cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名 / 全文")),
        size: Size::Medium,
    });
    for size in [Size::XSmall, Size::Small, Size::Medium, Size::Large] {
        for value in [
            "",
            "中文搜索内容 search",
            "很长的搜索文字 repeated search text",
        ] {
            probe.update(visual, |probe, cx| {
                probe.size = size;
                cx.notify();
            });
            visual.update(|window, cx| {
                let state = probe.read(cx).state.clone();
                state.update(cx, |state, cx| {
                    state.set_value(value, window, cx);
                    state.focus(window, cx);
                });
                window.draw(cx).clear(cx);
                let state = state.read(cx);
                let bounds = state.input_bounds();
                assert!(
                    bounds.size.height >= state.line_height().unwrap(),
                    "{size:?}: {bounds:?}"
                );
                assert!(bounds.size.width > px(0.));
                let caret = state.cursor_layout().unwrap().0;
                assert!(caret.top() >= bounds.top());
                assert!(caret.bottom() <= bounds.bottom());
            });
        }
    }
}
#[gpui::test]
fn replace_command_opens_the_focused_view_in_editing_mode(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("note.md".into(), Some("text".into()), false, window, cx);
            w.split_active(false, window, cx);
            w.focus_secondary(cx);
            let primary = w.tabs[0].pane.clone();
            let secondary = w.current_pane().unwrap();
            assert_ne!(primary, secondary);
            for pane in [&primary, &secondary] {
                pane.update(cx, |pane, _| pane.reading = true);
            }
            w.execute_command(23, window, cx);
            assert!(primary.read(cx).reading);
            assert_eq!(
                primary
                    .read(cx)
                    .editor
                    .read(cx)
                    .search_activation_revision(),
                0
            );
            assert!(!secondary.read(cx).reading);
            assert_eq!(
                secondary
                    .read(cx)
                    .editor
                    .read(cx)
                    .search_activation_revision(),
                1
            );
            assert!(
                secondary
                    .read(cx)
                    .editor
                    .read(cx)
                    .search_session()
                    .replace_mode
            );
            w.focus_primary(0, window, cx);
            w.execute_command(23, window, cx);
            assert!(!primary.read(cx).reading);
            assert_eq!(
                primary
                    .read(cx)
                    .editor
                    .read(cx)
                    .search_activation_revision(),
                1
            );
            assert_eq!(
                secondary
                    .read(cx)
                    .editor
                    .read(cx)
                    .search_activation_revision(),
                1
            );
        })
        .unwrap();
}

#[gpui::test]
fn live_task_click_updates_shared_views_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "- [ ] item\nend";
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("task.md".into(), Some(source.into()), false, window, cx);
            w.add_tab("task.md".into(), None, false, window, cx);
            w.current_pane()
                .unwrap()
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
            w.split_active(true, window, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let position = visual.debug_bounds("live-task-3").unwrap().center();
    visual.simulate_click(position, Modifiers::default());
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, _, cx| {
            for tab in &w.tabs {
                assert_eq!(
                    tab.pane.read(cx).editor.read(cx).value().as_ref(),
                    "- [x] item\nend"
                );
            }
            assert_eq!(
                w.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .as_ref(),
                "- [x] item\nend"
            );
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-z");
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, _, cx| {
            for tab in &w.tabs {
                assert_eq!(tab.pane.read(cx).editor.read(cx).value().as_ref(), source);
            }
            assert_eq!(
                w.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .as_ref(),
                source
            );
        })
        .unwrap();
}
#[gpui::test]
fn quick_font_wheel_updates_views_and_respects_modal(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab(
                "zoom.md".into(),
                Some("正文\n".repeat(100)),
                false,
                window,
                cx,
            );
            w.split_active(true, window, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let position = handle
        .update(&mut visual, |w, _, cx| {
            w.views
                .split
                .as_ref()
                .unwrap()
                .pane
                .read(cx)
                .editor
                .read(cx)
                .input_bounds()
                .center()
        })
        .unwrap();
    let wheel = |y| ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(y))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: TouchPhase::Moved,
    };
    visual.simulate_event(wheel(80.));
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(w.ui.prefs.font_size, 16.);
            w.ui.prefs.quick_font_size = true;
            w.apply_editor_preferences(window, cx);
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_event(wheel(80.));
    handle
        .update(&mut visual, |w, _, cx| {
            assert_eq!(w.ui.prefs.font_size, 17.);
            assert_eq!(w.tabs[0].pane.read(cx).font_size, 17.);
            assert_eq!(w.views.split.as_ref().unwrap().pane.read(cx).font_size, 17.);
            w.ui.settings = true;
            cx.notify();
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_event(wheel(80.));
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(w.ui.prefs.font_size, 17.);
            w.ui.settings = false;
            w.ui.prefs.font_size = 30.;
            w.views.split.as_ref().unwrap().pane.update(cx, |p, cx| {
                p.reading = true;
                cx.notify();
            });
            w.apply_editor_preferences(window, cx);
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_event(wheel(80.));
    handle
        .update(&mut visual, |w, _, _| assert_eq!(w.ui.prefs.font_size, 30.))
        .unwrap();
    visual.simulate_event(wheel(-80.));
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(w.ui.prefs.font_size, 29.);
            w.ui.prefs.font_size = 10.;
            w.apply_editor_preferences(window, cx);
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_event(wheel(-80.));
    handle
        .update(&mut visual, |w, _, _| assert_eq!(w.ui.prefs.font_size, 10.))
        .unwrap();
}

#[gpui::test]
fn immediate_save_and_window_close_flush_committed_linked_view_edits(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for close in [true, false] {
        let root = std::env::temp_dir().join(format!(
            "inkstone-immediate-view-save-{}-{close}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "abc").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("a.md".into(), Some("abc".into()), false, window, cx);
                w.execute_command(85, window, cx);
                w.tabs[1]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_selected_range(1..1, cx));
                w.persist_workspace(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                // Keep the caret unchanged so preference writes cannot mask a missed text change.
                w.tabs[1].pane.read(cx).editor.clone().update(cx, |s, cx| {
                    s.replace_text_in_range(Some(0..1), "X", window, cx)
                });
                if close {
                    assert!(!w.request_window_close(window, cx));
                } else {
                    w.save_all(window, cx);
                }
                assert_eq!(w.tabs[0].save.editor.read(cx).value(), "Xbc");
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "Xbc");
    }
}

#[gpui::test]
fn explicit_new_tab_link_events_open_unpinned_targets_independently(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let source = handle
        .update(cx, |w, window, cx| {
            let mut index = Index::default();
            index.update("a.md".into(), "source".into());
            index.update("b.md".into(), "intro\n\n# Target\nbody".into());
            w.index = Arc::new(index);
            w.add_tab("a.md".into(), Some("source".into()), false, window, cx);
            let source = w.tabs[0].pane.clone();
            w.add_tab(
                "b.md".into(),
                Some("intro\n\n# Target\nbody".into()),
                false,
                window,
                cx,
            );
            source
        })
        .unwrap();
    for (i, event) in [
        EditorEvent::FollowLinkInNewTab("b#Target".into()),
        EditorEvent::FollowMarkdownLinkInNewTab("b.md#Target".into()),
        EditorEvent::FollowReferenceInNewTab(inkstone_core::rendering::Reference {
            from: "a.md".into(),
            target: "b#Target".into(),
            wiki: true,
        }),
    ]
    .into_iter()
    .enumerate()
    {
        source.update(cx, |_, cx| cx.emit(event));
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.tabs.len(), 3 + i);
                assert_eq!(w.active, Some(2 + i));
                assert!(!w.tabs[0].pinned);
                assert!(std::rc::Rc::ptr_eq(&w.tabs[1].save, &w.tabs[2 + i].save));
                assert_eq!(
                    w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                    0..0
                );
                assert_eq!(
                    w.tabs[2 + i].pane.read(cx).editor.read(cx).selected_range(),
                    7..7
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn same_document_links_keep_the_current_duplicate_and_split(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let source = "intro\n\n# Target\nbody";
    let duplicate = handle
        .update(cx, |w, window, cx| {
            Arc::make_mut(&mut w.index).update("a.md".into(), source.into());
            w.add_tab("a.md".into(), Some(source.into()), false, window, cx);
            w.add_tab("a.md".into(), None, false, window, cx);
            let pane = w.tabs[1].pane.clone();
            pane.update(cx, |_, cx| {
                cx.emit(EditorEvent::FollowLink("a#Target".into()))
            });
            pane
        })
        .unwrap();
    cx.run_until_parked();
    let split = handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.active, Some(1));
            assert_eq!(w.current_pane().unwrap().entity_id(), duplicate.entity_id());
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).selected_range(),
                0..0
            );
            assert_eq!(duplicate.read(cx).editor.read(cx).selected_range(), 7..7);
            w.split_active(true, window, cx);
            let pane = w.current_pane().unwrap();
            pane.read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.set_selected_range(0..0, cx));
            pane.update(cx, |_, cx| {
                cx.emit(EditorEvent::FollowMarkdownLink("#Target".into()))
            });
            pane
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.tabs.len(), 2);
            assert!(w.views.secondary_focused);
            assert_eq!(w.active, Some(1));
            assert_eq!(w.current_pane().unwrap().entity_id(), split.entity_id());
            assert_eq!(split.read(cx).editor.read(cx).selected_range(), 7..7);
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).selected_range(),
                0..0
            );
        })
        .unwrap();
}

#[gpui::test]
fn pinned_link_opens_an_independent_target_and_keeps_existing_selection(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let original = handle
        .update(cx, |w, window, cx| {
            let mut index = Index::default();
            index.update("a.md".into(), "[[b#Target]]".into());
            index.update("b.md".into(), "intro\n\n# Target\nbody".into());
            w.index = Arc::new(index);
            w.add_tab(
                "a.md".into(),
                Some("[[b#Target]]".into()),
                false,
                window,
                cx,
            );
            let original = w.tabs[0].pane.clone();
            w.add_tab(
                "b.md".into(),
                Some("intro\n\n# Target\nbody".into()),
                false,
                window,
                cx,
            );
            w.tabs[1]
                .pane
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.set_selected_range(0..0, cx));
            w.split_active(false, window, cx);
            w.focus_primary(0, window, cx);
            w.execute_command(27, window, cx);
            w.focus_secondary(cx);
            original.update(cx, |_, cx| {
                cx.emit(EditorEvent::FollowLink("b#Target".into()))
            });
            original
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.tabs.len(), 3);
            assert_eq!(w.active, Some(2));
            assert!(w.tabs[0].pinned);
            assert_eq!(w.tabs[0].pane.entity_id(), original.entity_id());
            assert!(std::rc::Rc::ptr_eq(&w.tabs[1].save, &w.tabs[2].save));
            assert!(!w.tabs[2].pinned);
            assert_eq!(
                w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                0..0
            );
            assert_eq!(
                w.tabs[2].pane.read(cx).editor.read(cx).selected_range(),
                7..7
            );
        })
        .unwrap();
}

#[gpui::test]
fn pinned_split_link_preserves_source_while_loading_new_target(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-pinned-link-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "source").unwrap();
    std::fs::write(root.join("b.md"), "intro\n\n# Target\nbody").unwrap();
    let handle = cx.add_window(Workspace::new);
    let source = handle
        .update(cx, |w, window, cx| {
            let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
            w.index = Arc::new(Index::build(&vault).unwrap());
            w.vault = Some(vault);
            w.add_tab("a.md".into(), Some("source".into()), false, window, cx);
            w.split_active(false, window, cx);
            w.execute_command(27, window, cx);
            let source = w.views.split.as_ref().unwrap().pane.entity_id();
            w.follow_markdown_link("a.md".into(), "b.md#Target", false, window, cx);
            assert!(w.views.secondary_focused);
            source
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.active, Some(1));
            assert!(!w.views.secondary_focused);
            let split = w.views.split.as_ref().unwrap();
            assert_eq!(split.pane.entity_id(), source);
            assert!(split.pinned);
            assert_eq!(split.pane.read(cx).editor.read(cx).value(), "source");
            assert_eq!(
                w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                7..7
            );
        })
        .unwrap();
}

#[gpui::test]
fn pinned_views_migrate_and_restore_independently(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-view-pins-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let prefs = inkstone_core::preferences::Preferences {
        open_paths: vec!["a.md".into(), "a.md".into()],
        pinned_paths: vec!["a.md".into()],
        active_tab_index: Some(1),
        views: vec![
            inkstone_core::preferences::ViewState {
                path: "a.md".into(),
                ..Default::default()
            },
            inkstone_core::preferences::ViewState {
                path: "a.md".into(),
                pinned: Some(false),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    prefs.save(&root.join(".inkstone-workspace.json")).unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.tabs[0].pinned);
            assert!(!w.tabs[1].pinned);
            assert!(w.ui.prefs.pinned_paths.is_empty());
            w.focus_primary(0, window, cx);
            w.execute_command(27, window, cx);
            w.focus_primary(1, window, cx);
            w.execute_command(27, window, cx);
            assert!(!w.tabs[0].pinned);
            assert!(w.tabs[1].pinned);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(!w.tabs[0].pinned);
            assert!(w.tabs[1].pinned);
            w.close_tab_at(0, window, cx);
            assert_eq!(w.tabs.len(), 1);
            w.close_tab_at(0, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert!(!w.tabs[0].pinned);
            w.close_tab_at(0, window, cx);
            assert!(w.tabs.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn split_pin_is_independent_and_survives_promotion(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.execute_command(27, window, cx);
            w.split_active(false, window, cx);
            assert!(w.tabs[0].pinned);
            assert!(!w.views.split.as_ref().unwrap().pinned);
            w.execute_command(27, window, cx);
            w.close_tab(window, cx);
            assert!(!w.views.split.as_ref().unwrap().pinned);
            assert!(w.tabs[0].pinned);
            w.focus_primary(0, window, cx);
            w.close_tab(window, cx);
            assert!(!w.tabs[0].pinned);
            w.focus_secondary(cx);
            w.execute_command(27, window, cx);
            w.focus_primary(0, window, cx);
            w.close_tab_at(0, window, cx);
            assert!(w.views.split.is_none());
            assert!(w.tabs[0].pinned);
        })
        .unwrap();
}

#[gpui::test]
fn session_restores_duplicate_note_views_and_split_after_missing_file(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root =
        std::env::temp_dir().join(format!("inkstone-duplicate-session-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "甲\n乙").unwrap();
    let prefs = inkstone_core::preferences::Preferences {
        open_paths: vec!["missing.md".into(), "a.md".into(), "a.md".into()],
        active_path: Some("a.md".into()),
        active_tab_index: Some(2),
        main_path: Some("a.md".into()),
        main_tab_index: Some(1),
        split_source_tab_index: Some(2),
        views: vec![
            inkstone_core::preferences::ViewState {
                path: "missing.md".into(),
                ..Default::default()
            },
            inkstone_core::preferences::ViewState {
                path: "a.md".into(),
                reading: true,
                selection: 0..0,
                ..Default::default()
            },
            inkstone_core::preferences::ViewState {
                path: "a.md".into(),
                live: false,
                selection: 4..4,
                ..Default::default()
            },
        ],
        split_view: Some(inkstone_core::preferences::ViewState {
            path: "a.md".into(),
            selection: 7..7,
            pinned: Some(true),
            ..Default::default()
        }),
        split_focused: true,
        ..Default::default()
    };
    prefs.save(&root.join(".inkstone-workspace.json")).unwrap();
    let handle = cx.add_window(Workspace::new);
    for _ in 0..2 {
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.tabs.len(), 2);
                assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
                assert!(w.tabs[0].pane.read(cx).reading);
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).selected_range(),
                    0..0
                );
                assert!(!w.tabs[1].pane.read(cx).reading);
                assert!(!w.tabs[1].pane.read(cx).live);
                assert_eq!(
                    w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                    4..4
                );
                assert_eq!(w.active, Some(1));
                assert_eq!(w.main_tab(), Some(0));
                let split = w.views.split.as_ref().unwrap();
                assert_eq!(split.source, w.tabs[1].id);
                assert!(split.pinned);
                assert_eq!(split.pane.read(cx).editor.read(cx).selected_range(), 7..7);
                assert!(w.views.secondary_focused);
            })
            .unwrap();
    }
    let saved =
        inkstone_core::preferences::Preferences::load(&root.join(".inkstone-workspace.json"));
    assert_eq!(saved.main_tab_index, Some(0));
    assert_eq!(saved.split_source_tab_index, Some(1));
    assert_eq!(saved.open_paths.len(), 2);
}

#[gpui::test]
fn new_tab_command_shares_document_and_keeps_independent_view_state(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-new-view-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "甲\n乙").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("甲\n乙".into()), false, window, cx);
            w.tabs[0].pane.update(cx, |p, cx| {
                p.reading = true;
                p.editor.update(cx, |s, cx| s.set_selected_range(4..4, cx));
            });
            w.execute_command(85, window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
            assert_ne!(w.tabs[0].pane.entity_id(), w.tabs[1].pane.entity_id());
            w.tabs[1].pane.update(cx, |p, cx| {
                assert!(p.reading);
                assert_eq!(p.editor.read(cx).selected_range(), 4..4);
                p.reading = false;
                p.editor
                    .update(cx, |s, cx| s.replace_text_in_range(None, "X", window, cx));
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.tabs[0].pane.read(cx).reading);
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).selected_range(),
                4..4
            );
            assert_eq!(w.tabs[0].save.editor.read(cx).value(), "甲\nX乙");
            w.save_all(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "甲\nX乙"
    );
    handle
        .update(cx, |w, window, cx| {
            w.close_tab_at(1, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert!(w.tabs[0].pane.read(cx).reading);
        })
        .unwrap();
}

#[gpui::test]
fn new_tab_from_split_keeps_split_and_rejects_pending_composition(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("one\ntwo".into()), false, window, cx);
            w.split_active(false, window, cx);
            let split = w.views.split.as_ref().unwrap().pane.clone();
            split.update(cx, |p, cx| {
                p.live = false;
                p.editor.update(cx, |s, cx| s.set_selected_range(5..5, cx));
            });
            w.execute_command(85, window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(
                w.views.split.as_ref().unwrap().pane.entity_id(),
                split.entity_id()
            );
            assert!(!w.views.secondary_focused);
            assert!(!w.tabs[1].pane.read(cx).live);
            assert_eq!(
                w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                5..5
            );
            split.read(cx).editor.clone().update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
            });
            w.execute_command(85, window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert!(w.ui.closed.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn closed_shared_view_reopens_without_changing_the_surviving_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.tabs[0].pane.update(cx, |p, cx| {
                p.reading = true;
                p.editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
            });
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let survivor = w.tabs[1].pane.clone();
            survivor
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.set_selected_range(6..6, cx));
            w.close_tab_at(0, window, cx);
            let editor = survivor.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "你", Some(1..1), window, cx)
            });
            w.execute_command(17, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.ui.closed.len(), 1);
            editor.update(cx, |s, cx| s.replace_text_in_range(None, "", window, cx));
            w.execute_command(17, window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[0].pane.entity_id(), survivor.entity_id());
            assert!(!survivor.read(cx).reading);
            assert_eq!(editor.read(cx).selected_range(), 6..6);
            assert!(w.tabs[1].pane.read(cx).reading);
            assert_eq!(
                w.tabs[1].pane.read(cx).editor.read(cx).selected_range(),
                3..3
            );
            assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
        })
        .unwrap();
}

#[gpui::test]
fn closing_shared_view_keeps_dirty_document_until_last_view_saves(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-close-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.tabs[1].pane.read(cx).editor.clone().update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_text_in_range(None, "X", window, cx);
            });
            w.tabs[0].save.persistence.test_set_conflict(true);
            w.tabs[0]
                .save
                .persistence
                .test_set_error(Some("验收冲突".into()));
            w.close_tab_at(1, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.tabs[0].save.editor.read(cx).value(), "原文X");
            assert!(w.tabs[0].save.persistence.is_dirty());
            assert!(w.tabs[0].save.persistence.has_conflict());
            w.close_tab_at(0, window, cx);
            assert_eq!(w.tabs.len(), 1);
            w.tabs[0].save.persistence.test_set_conflict(false);
            w.tabs[0].save.persistence.test_set_error(None);
            w.close_tab_at(0, window, cx);
            assert_eq!(w.tabs.len(), 1, "last view waits for its save");
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| assert!(w.tabs.is_empty()))
        .unwrap();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "原文X");
}

#[gpui::test]
fn split_survives_source_view_close_and_keeps_editing_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for with_peer in [false, true] {
        let handle = cx.add_window(Workspace::new);
        let editor = handle
            .update(cx, |w, window, cx| {
                w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
                if with_peer {
                    w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
                }
                w.focus_primary(0, window, cx);
                w.split_active(false, window, cx);
                let pane = w.views.split.as_ref().unwrap().pane.clone();
                let editor = pane.read(cx).editor.clone();
                pane.update(cx, |p, _| p.live = false);
                editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
                w.focus_primary(0, window, cx);
                w.close_tab_at(0, window, cx);
                assert_eq!(w.tabs.len(), 1);
                if with_peer {
                    let split = w.views.split.as_ref().unwrap();
                    assert_eq!(split.source, w.tabs[0].id);
                    assert_eq!(split.pane.entity_id(), pane.entity_id());
                } else {
                    assert!(w.views.split.is_none());
                    assert_eq!(w.tabs[0].pane.entity_id(), pane.entity_id());
                }
                assert!(!pane.read(cx).live);
                assert_eq!(editor.read(cx).selected_range(), 3..3);
                editor.update(cx, |s, cx| s.focus(window, cx));
                editor
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("x");
        visual.run_until_parked();
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "原x文"));
        handle
            .update(&mut visual, |w, _, cx| {
                assert_eq!(w.tabs[0].save.editor.read(cx).value(), "原x文")
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        visual.run_until_parked();
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "原文"));
    }
}

#[gpui::test]
fn shared_document_paths_follow_rename_and_recovery_copy(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-path-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let mirror = w.tabs[1].pane.read(cx).editor.clone();
            mirror.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
            });
            w.manage_named_note(w.tabs[0].id, false, "renamed.md".into(), window, cx);
            assert!(!w.ui.file_operation);
            mirror.update(cx, |s, cx| s.replace_text_in_range(None, "", window, cx));
            w.manage_named_note(w.tabs[0].id, false, "renamed.md".into(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!root.join("a.md").exists());
    handle
        .update(cx, |w, window, cx| {
            assert!(
                w.tabs
                    .iter()
                    .all(|tab| tab.path == std::path::Path::new("renamed.md"))
            );
            assert_eq!(*w.tabs[0].save.path.borrow(), PathBuf::from("renamed.md"));
            w.tabs[1].pane.read(cx).editor.clone().update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_text_in_range(None, "X", window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    let copy = handle
        .update(cx, |w, window, cx| {
            w.save_copy(window, cx);
            let path = w.tabs[0].path.clone();
            assert!(w.tabs.iter().all(|tab| tab.path == path));
            assert_eq!(*w.tabs[0].save.path.borrow(), path);
            path
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join(&copy)).unwrap(), "原文X");
    assert_eq!(
        std::fs::read_to_string(root.join("renamed.md")).unwrap(),
        "原文"
    );
}

#[gpui::test]
fn shared_document_rename_survives_origin_view_removal(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-rename-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.manage_named_note(w.tabs[0].id, false, "b.md".into(), window, cx);
            w.tabs.remove(0);
            w.active = Some(0);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert_eq!(w.tabs[0].path, PathBuf::from("b.md"));
            assert_eq!(*w.tabs[0].save.path.borrow(), PathBuf::from("b.md"));
            assert!(!w.tabs[0].save.persistence.is_saving());
        })
        .unwrap();
    assert_eq!(std::fs::read_to_string(root.join("b.md")).unwrap(), "原文");
}

#[gpui::test]
fn trashing_shared_document_closes_all_its_views(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-trash-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.split_active(false, window, cx);
            w.manage_named_note(w.tabs[0].id, true, String::new(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert!(w.tabs.is_empty());
            assert!(w.views.split.is_none());
            assert!(w.views.main.is_none());
        })
        .unwrap();
    assert!(!root.join("a.md").exists());
}

#[gpui::test]
fn external_reload_updates_the_shared_owner_after_its_first_view_closes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-reload-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.tabs.remove(0);
            w.active = Some(0);
            std::fs::write(root.join("a.md"), "外部修改").unwrap();
            w.refresh(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            let tab = &w.tabs[0];
            assert_eq!(tab.save.editor.read(cx).value(), "外部修改");
            assert_eq!(tab.pane.read(cx).editor.read(cx).value(), "外部修改");
            assert!(!tab.save.persistence.is_dirty());
            assert!(!tab.save.persistence.has_conflict());
        })
        .unwrap();
}

#[gpui::test]
fn shared_document_views_keep_history_after_owner_view_closes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let (owner, mirror) = handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("abc".into()), false, window, cx);
            let owner = w.tabs[0].pane.read(cx).editor.clone();
            owner.update(cx, |s, cx| s.set_selected_range(1..1, cx));
            w.add_tab(
                "a.md".into(),
                Some("stale disk text".into()),
                false,
                window,
                cx,
            );
            let mirror = w.tabs[1].pane.read(cx).editor.clone();
            assert_ne!(owner.entity_id(), mirror.entity_id());
            assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
            mirror.update(cx, |s, cx| {
                assert_eq!(s.value(), "abc");
                s.set_selected_range(3..3, cx);
                s.replace_text_in_range(None, "X", window, cx);
                s.focus(window, cx);
            });
            (owner, mirror)
        })
        .unwrap();
    cx.run_until_parked();
    owner.read_with(cx, |s, _| {
        assert_eq!(s.value(), "abcX");
        assert_eq!(s.selected_range(), 1..1);
    });
    handle
        .update(cx, |w, window, cx| {
            w.tabs.remove(0);
            w.active = Some(0);
            mirror.update(cx, |s, cx| s.focus(window, cx));
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-z");
    visual.run_until_parked();
    mirror.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "abc");
        assert_eq!(s.selected_range(), 3..3);
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    visual.run_until_parked();
    mirror.read_with(&visual, |s, _| assert_eq!(s.value(), "abcX"));
    visual.simulate_platform_keystrokes("y");
    visual.run_until_parked();
    owner.read_with(&visual, |s, _| assert_eq!(s.value(), "abcXy"));
    mirror.read_with(&visual, |s, _| assert_eq!(s.value(), "abcXy"));
}

#[gpui::test]
fn shared_document_save_survives_removing_the_originating_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-shared-save-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            for tab in &w.tabs {
                tab.pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("原文新", window, cx));
            }
            w.tabs[0].save.persistence.test_set_dirty(true);
            w.save_all(window, cx);
            assert!(w.tabs.iter().all(|tab| tab.save.persistence.is_saving()));
            // Closing one view must not orphan the document's pending save.
            w.tabs.remove(0);
            w.active = Some(0);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "原文新"
    );
    handle
        .update(cx, |w, window, cx| {
            let tab = &w.tabs[0];
            assert!(!tab.save.persistence.is_saving());
            assert!(!tab.save.persistence.is_dirty());
            assert!(!tab.save.persistence.has_conflict());
            assert!(tab.save.persistence.error().is_none());
            assert_eq!(tab.save.persistence.baseline().as_deref(), Some("原文新"));
            tab.pane.read(cx).editor.clone().update(cx, |s, cx| {
                s.select_all(window, cx);
                s.replace_text_in_range(None, "继续", window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| w.save_all(window, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "继续");
}

#[gpui::test]
fn closing_an_unchanged_split_cannot_revert_a_new_primary_edit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-stale-split-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let primary = w.tabs[0].pane.read(cx).editor.clone();
            w.split_active(false, window, cx);
            w.focus_primary(0, window, cx);
            primary.update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_text_in_range(None, "X", window, cx);
            });
            w.close_split(window, cx);
            assert!(w.views.split.is_none());
            assert_eq!(primary.read(cx).value(), "原文X");
            assert!(!w.request_window_close(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "原文X");
}

#[gpui::test]
fn unchanged_primary_cannot_overwrite_a_new_split_edit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let primary = w.tabs[0].pane.read(cx).editor.clone();
            w.split_active(false, window, cx);
            let mirror = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            mirror.update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_text_in_range(None, "X", window, cx);
            });
            w.sync_to_split(w.tabs[0].id, window, cx);
            assert_eq!(mirror.read(cx).value(), "原文X");
            w.close_split(window, cx);
            assert_eq!(primary.read(cx).value(), "原文X");
        })
        .unwrap();
}

#[gpui::test]
fn primary_composition_stays_local_until_commit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let (primary, mirror) = handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let primary = w.tabs[0].pane.read(cx).editor.clone();
            w.split_active(false, window, cx);
            let mirror = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            w.focus_primary(0, window, cx);
            primary.update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            });
            w.sync_to_split(w.tabs[0].id, window, cx);
            (primary, mirror)
        })
        .unwrap();
    cx.run_until_parked();
    mirror.read_with(cx, |s, _| assert_eq!(s.value(), "原文"));
    handle
        .update(cx, |_, window, cx| {
            primary.update(cx, |s, cx| s.replace_text_in_range(None, "", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    mirror.read_with(cx, |s, _| assert_eq!(s.value(), "原文"));
    handle
        .update(cx, |_, window, cx| {
            primary.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "你", Some(1..1), window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    mirror.read_with(cx, |s, _| assert_eq!(s.value(), "原文"));
    handle
        .update(cx, |_, window, cx| {
            primary.update(cx, |s, cx| s.unmark_text(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    mirror.read_with(cx, |s, _| assert_eq!(s.value(), "原文你"));
    handle
        .update(cx, |_, window, cx| {
            primary.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    mirror.read_with(cx, |s, _| assert_eq!(s.value(), "原文"));
}

#[gpui::test]
fn split_creation_waits_for_committed_text(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            let editor = w.tabs[0].pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_and_mark_text_in_range(None, "你", Some(1..1), window, cx);
            });
            w.split_active(false, window, cx);
            assert!(w.views.split.is_none());
            assert!(editor.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()));
            editor.update(cx, |s, cx| s.unmark_text(window, cx));
            w.split_active(false, window, cx);
            let split = w.views.split.as_ref().unwrap();
            assert_eq!(split.pane.read(cx).editor.read(cx).value(), "原文你");
        })
        .unwrap();
}

#[gpui::test]
fn split_unmark_commits_and_saves_without_an_additional_text_edit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-split-unmark-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    let handle = cx.add_window(Workspace::new);
    let editor = handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.split_active(false, window, cx);
            let editor = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                s.set_selected_range(6..6, cx);
                s.replace_and_mark_text_in_range(None, "你", Some(1..1), window, cx);
            });
            w.save_all(window, cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "原文");
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), "原文");
            editor.update(cx, |s, cx| s.unmark_text(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), "原文你");
            assert!(w.tabs[0].save.persistence.is_dirty());
            w.save_all(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "原文你"
    );
    handle
        .update(cx, |w, window, cx| {
            let owner = w.tabs[0].pane.read(cx).editor.clone();
            owner.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    editor.read_with(cx, |s, _| assert_eq!(s.value(), "原文"));
}
#[gpui::test]
fn split_undo_groups_continuous_typing_but_keeps_paste_separate(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let editor = handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some(String::new()), false, window, cx);
            w.split_active(false, window, cx);
            w.views.split.as_ref().unwrap().pane.read(cx).editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("a b c");
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "abc"));
    visual.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("X".into())));
    visual.simulate_platform_keystrokes("ctrl-v");
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "abcX"));
    visual.simulate_platform_keystrokes("ctrl-z");
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "abc"));
    visual.simulate_platform_keystrokes("ctrl-z");
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), ""));
    visual.simulate_platform_keystrokes("ctrl-y ctrl-y");
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, _, cx| {
            assert_eq!(editor.read(cx).value(), "abcX");
            assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), "abcX");
        })
        .unwrap();
}
#[gpui::test]
fn split_multiple_carets_survive_undo_redo_and_further_input(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let editor = handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("中\n文".into()), false, window, cx);
            w.split_active(false, window, cx);
            let editor = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                s.set_selected_range(3..3, cx);
                s.focus(window, cx);
            });
            editor
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down");
    visual.simulate_input("😀");
    visual.run_until_parked();
    visual.simulate_platform_keystrokes("ctrl-z");
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "中\n文");
        assert!(s.has_multiple_selections());
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    visual.run_until_parked();
    visual.simulate_input("Y");
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, _, cx| {
            assert_eq!(editor.read(cx).value(), "中😀Y\n文😀Y");
            assert!(editor.read(cx).has_multiple_selections());
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).value(),
                "中😀Y\n文😀Y"
            );
            assert!(w.views.secondary_focused);
        })
        .unwrap();
}
#[gpui::test]
fn background_counts_from_split_do_not_change_active_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab(
                "long.md".into(),
                Some("word ".repeat(3000)),
                false,
                window,
                cx,
            );
            w.split_active(false, window, cx);
            w.views.split.as_ref().unwrap().pane.update(cx, |p, cx| {
                p.text_counts(cx);
            });
            w.focus_primary(0, window, cx);
            assert!(!w.views.secondary_focused);
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(250));
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| assert!(!w.views.secondary_focused))
        .unwrap();
}
#[gpui::test]
fn fonts_restore_across_vault_and_theme_changes(cx: &mut TestAppContext) {
    use inkstone_core::preferences::{Preferences, ThemeMode};
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-font-session-{}", std::process::id()));
    let first = root.join("first");
    let second = root.join("second");
    for vault in [&first, &second] {
        std::fs::create_dir_all(vault).unwrap();
        std::fs::write(vault.join("note.md"), "中文😀\n原文").unwrap();
    }
    let handle = cx.add_window(Workspace::new);
    let font = handle
        .update(cx, |w, _, _| {
            let font = w.ui.default_fonts.0.to_string();
            // TestPlatform has no font enumeration; use its already resolved interface family.
            w.ui.available_fonts = Arc::new(vec![font.clone()]);
            font
        })
        .unwrap();
    for (vault, custom) in [(&first, true), (&second, false)] {
        Preferences {
            theme: if custom {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            interface_font: if custom { font.clone() } else { String::new() },
            text_font: if custom { font.clone() } else { String::new() },
            monospace_font: if custom { font.clone() } else { String::new() },
            open_paths: vec!["note.md".into()],
            ..Default::default()
        }
        .save(&vault.join(".inkstone-workspace.json"))
        .unwrap();
    }
    for (vault, custom) in [(&first, true), (&second, false), (&first, true)] {
        handle
            .update(cx, |w, window, cx| w.load_vault(vault.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs.len(), 1);
                let expected = if custom { font.as_str() } else { "" };
                for selector in &w.ui.font_selects {
                    assert_eq!(
                        selector.read(cx).selected_value().map(String::as_str),
                        Some(expected)
                    );
                }
                assert_eq!(w.ui.prefs.text_font, expected);
                assert_eq!(
                    w.tabs[0].pane.read(cx).text_font,
                    w.resolved_font(expected, "Microsoft YaHei UI")
                );
                w.execute_command(22, window, cx);
                assert_eq!(
                    w.tabs[0].pane.read(cx).text_font,
                    w.resolved_font(expected, "Microsoft YaHei UI")
                );
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                    "中文😀\n原文"
                );
                w.watcher = None;
                w.persist_workspace(cx);
            })
            .unwrap();
        cx.run_until_parked();
        let saved = Preferences::load(&vault.join(".inkstone-workspace.json"));
        assert_eq!(saved.text_font, if custom { font.as_str() } else { "" });
        assert_eq!(
            std::fs::read_to_string(vault.join("note.md")).unwrap(),
            "中文😀\n原文"
        );
    }
}
#[gpui::test]
fn font_search_supports_mouse_and_keyboard_without_crossing_roles(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let name = handle
        .update(cx, |w, window, cx| {
            w.ui.settings = true;
            w.ui.settings_tab = 5;
            let name = ".SystemUIFont".to_string();
            // TestPlatform does not enumerate installed fonts; the virtual system family is portable.
            w.ui.available_fonts = Arc::new(vec![name.clone()]);
            for select in &w.ui.font_selects {
                select.update(cx, |state, cx| {
                    state.set_items(
                        gpui_component::select::SearchableVec::new(vec![
                            appearance::FontChoice {
                                name: String::new(),
                                missing: false,
                            },
                            appearance::FontChoice {
                                name: name.clone(),
                                missing: false,
                            },
                        ]),
                        window,
                        cx,
                    );
                });
            }
            window.focus(&w.ui.font_selects[1].read(cx).focus_handle(cx), cx);
            name
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1100.), px(800.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let trigger = visual.debug_bounds("font-text-trigger").unwrap();
    assert_eq!(trigger.size.width, px(240.));
    let content = handle
        .update(&mut visual, |w, _, _| w.ui.settings_scroll.bounds())
        .unwrap();
    assert!(trigger.left() >= content.center().x);
    visual.simulate_click(trigger.center(), Modifiers::default());
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_input(&name);
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(w.ui.prefs.text_font, name);
            assert!(w.ui.prefs.interface_font.is_empty());
            assert!(w.ui.prefs.monospace_font.is_empty());
            window.focus(&w.ui.font_selects[0].read(cx).focus_handle(cx), cx);
        })
        .unwrap();
    visual.simulate_platform_keystrokes("down");
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_input("默认");
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    handle
        .update(&mut visual, |w, _, _| {
            assert!(w.ui.prefs.interface_font.is_empty());
            assert_eq!(w.ui.prefs.text_font, name);
        })
        .unwrap();
}
#[gpui::test]
fn font_preferences_reach_split_views_and_missing_fonts_fall_back(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let chosen = w.ui.available_fonts.first().cloned().unwrap_or_default();
            w.ui.prefs.interface_font = chosen.clone();
            w.ui.prefs.text_font = chosen.clone();
            w.ui.prefs.monospace_font = chosen;
            w.add_tab("a.md".into(), Some("中文😀".into()), false, window, cx);
            w.split_active(false, window, cx);
            let expected = w.resolved_font(&w.ui.prefs.text_font, "Microsoft YaHei UI");
            assert_eq!(w.tabs[0].pane.read(cx).text_font, expected);
            assert_eq!(
                w.views.split.as_ref().unwrap().pane.read(cx).text_font,
                expected
            );
            assert_eq!(
                gpui_component::Theme::global(cx).font_family,
                w.resolved_font(&w.ui.prefs.interface_font, &w.ui.default_fonts.0)
            );
            assert_eq!(
                gpui_component::Theme::global(cx).mono_font_family,
                w.resolved_font(&w.ui.prefs.monospace_font, &w.ui.default_fonts.1)
            );
            w.ui.prefs.text_font = "inkstone-nonexistent-font-123".into();
            w.apply_editor_preferences(window, cx);
            assert_eq!(
                w.tabs[0].pane.read(cx).text_font.as_ref(),
                "Microsoft YaHei UI"
            );
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                "中文😀"
            );
        })
        .unwrap();
}
#[gpui::test]
fn system_theme_updates_views_but_respects_manual_override(cx: &mut TestAppContext) {
    use inkstone_core::preferences::ThemeMode;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("# A".into()), false, window, cx);
            w.split_active(false, window, cx);
            w.set_theme(ThemeMode::Dark, window, cx);
            w.update_system_theme(true, window, cx);
            assert!(!w.ui.prefs.light);
            w.set_theme(ThemeMode::System, window, cx);
            for light in [false, true] {
                w.update_system_theme(light, window, cx);
                assert_eq!(w.ui.prefs.light, light);
                assert_eq!(w.tabs[0].pane.read(cx).light, light);
                assert_eq!(w.views.split.as_ref().unwrap().pane.read(cx).light, light);
            }
            w.execute_command(22, window, cx);
            assert_eq!(w.ui.prefs.theme, ThemeMode::Dark);
            w.update_system_theme(true, window, cx);
            assert!(!w.ui.prefs.light);
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                "# A"
            );
        })
        .unwrap();
}
#[gpui::test]
fn settings_content_scrolls_inside_short_windows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([KeyBinding::new("escape", ClosePalette, None)]);
    });
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.ui.settings = true;
            window.focus(&w.ui.modal_focus, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1000.), px(420.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |w, _, _| {
            let scroll = &w.ui.settings_scroll;
            let bounds = scroll.bounds();
            assert!(
                bounds.size.height > px(80.) && bounds.bottom() <= px(404.),
                "{bounds:?}"
            );
            assert!(
                scroll.max_offset().y > px(0.),
                "settings must overflow vertically"
            );
            scroll.scroll_to_bottom();
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |w, _, _| {
            let scroll = &w.ui.settings_scroll;
            let last = scroll.bounds_for_item(scroll.children_count() - 1).unwrap();
            assert!(last.bottom() + scroll.offset().y <= scroll.bounds().bottom() + px(1.));
            assert!(last.top() + scroll.offset().y >= scroll.bounds().top());
        })
        .unwrap();
    for tab in 1..7 {
        handle
            .update(&mut visual, |w, _, _| {
                w.ui.settings_tab = tab;
                w.ui.settings_scroll.set_offset(Point::default());
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |w, _, _| {
                let bounds = w.ui.settings_scroll.bounds();
                assert!(
                    bounds.size.height > px(0.) && bounds.bottom() <= px(404.),
                    "tab {tab}: {bounds:?}"
                );
            })
            .unwrap();
    }
    visual.simulate_platform_keystrokes("escape");
    handle
        .update(&mut visual, |w, _, _| assert!(!w.ui.settings))
        .unwrap();
}
#[gpui::test]
fn editor_settings_update_existing_and_new_split_views(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("- 中文".into()), false, window, cx);
            let a = w.current_pane().unwrap();
            w.ui.prefs.use_tabs = false;
            w.ui.prefs.tab_size = 6;
            w.apply_editor_preferences(window, cx);
            w.split_active(false, window, cx);
            assert_eq!(
                w.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .read(cx)
                    .indentation
                    .tab_size,
                6
            );
            w.ui.tab_width.update(cx, |_, cx| {
                cx.emit(gpui_component::slider::SliderEvent::Change(2f32.into()))
            });
            w.ui.font_size_slider.update(cx, |_, cx| {
                cx.emit(gpui_component::slider::SliderEvent::Change(10f32.into()))
            });
            assert_eq!(a.read(cx).editor.read(cx).value().as_ref(), "- 中文");
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.ui.prefs.tab_size, 2);
            assert_eq!(w.ui.prefs.font_size, 10.);
            for pane in [&w.tabs[0].pane, &w.views.split.as_ref().unwrap().pane] {
                assert_eq!(pane.read(cx).indentation.tab_size, 2);
                assert!(!pane.read(cx).indentation.hard_tabs);
                assert_eq!(pane.read(cx).font_size, 10.);
            }
        })
        .unwrap();
}
#[gpui::test]
fn default_view_applies_to_opened_notes_but_preserves_new_and_restored_views(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("# A".into()), false, window, cx);
            let a = w.current_pane().unwrap();
            w.set_default_view(true, cx);
            assert!(!a.read(cx).reading);
            w.add_tab("b.md".into(), Some("# B".into()), false, window, cx);
            let b = w.current_pane().unwrap();
            assert!(b.read(cx).reading);
            w.split_active(false, window, cx);
            assert!(w.views.split.as_ref().unwrap().pane.read(cx).reading);
            w.set_default_view(false, cx);
            assert!(b.read(cx).reading);
            assert!(w.views.split.as_ref().unwrap().pane.read(cx).reading);
            w.set_default_view(true, cx);
            Workspace::restore_view_state(
                &b,
                &inkstone_core::preferences::ViewState {
                    reading: false,
                    ..Default::default()
                },
                cx,
            );
            assert!(!b.read(cx).reading);
            w.add_tab("new.md".into(), None, true, window, cx);
            assert!(!w.current_pane().unwrap().read(cx).reading);
            assert_eq!(a.read(cx).editor.read(cx).value().as_ref(), "# A");
            assert_eq!(b.read(cx).editor.read(cx).value().as_ref(), "# B");
        })
        .unwrap();
}
#[gpui::test]
fn default_editing_mode_updates_open_views_and_preserves_reading_state(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.ui.prefs.default_live_preview = false;
            w.add_tab("a.md".into(), Some("# A".into()), false, window, cx);
            let a = w.current_pane().unwrap();
            assert!(!a.read(cx).live);
            w.execute_command(7, window, cx);
            w.apply_editor_preferences(window, cx);
            assert!(a.read(cx).live);
            w.add_tab("b.md".into(), Some("# B".into()), false, window, cx);
            assert!(!w.current_pane().unwrap().read(cx).live);
            assert!(a.read(cx).live);
            w.split_active(false, window, cx);
            a.update(cx, |pane, _| pane.reading = true);
            w.set_default_editing_mode(true, cx);
            assert!(a.read(cx).reading && a.read(cx).live);
            assert!(w.views.split.as_ref().unwrap().pane.read(cx).live);
            w.set_default_editing_mode(false, cx);
            assert!(a.read(cx).reading && !a.read(cx).live);
            assert!(!w.views.split.as_ref().unwrap().pane.read(cx).live);
            Workspace::restore_view_state(
                &a,
                &inkstone_core::preferences::ViewState {
                    reading: true,
                    live: true,
                    ..Default::default()
                },
                cx,
            );
            assert!(a.read(cx).live);
            assert!(!w.ui.prefs.default_live_preview);
            assert_eq!(a.read(cx).editor.read(cx).value().as_ref(), "# A");
        })
        .unwrap();
}
#[gpui::test]
fn search_validation_reports_inline_errors_without_overwriting_other_status(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.fulltext = true;
            w.search
                .update(cx, |s, cx| s.set_value("/[bad/", window, cx));
            w.run_search(cx);
            assert!(!w.ui.search_error.is_empty());
            assert_eq!(w.status, w.ui.search_error);
            assert!(!w.ui.search_loading);
            assert!(!w.ui.search_has_more);
            w.search
                .update(cx, |s, cx| s.set_value("valid", window, cx));
            w.run_search(cx);
            assert!(w.ui.search_error.is_empty());
            assert!(w.status.is_empty());
            w.search
                .update(cx, |s, cx| s.set_value("(broken", window, cx));
            w.run_search(cx);
            assert!(!w.ui.search_error.is_empty());
            w.status = "保存冲突待处理".into();
            w.search
                .update(cx, |s, cx| s.set_value("valid", window, cx));
            w.run_search(cx);
            assert!(w.ui.search_error.is_empty());
            assert_eq!(w.status, "保存冲突待处理");
        })
        .unwrap();
}
#[gpui::test]
fn search_loads_beyond_two_hundred_and_resets_for_new_query(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            Arc::make_mut(&mut w.index).update("many.md".into(), "hit\n".repeat(450));
            w.fulltext = true;
            w.search.update(cx, |s, cx| s.set_value("hit", window, cx));
            w.run_search(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.search_results.len(), 200);
            assert!(w.ui.search_has_more);
            w.load_more_search(cx);
            w.load_more_search(cx);
            assert_eq!(w.ui.search_limit, 400);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.search_results.len(), 400);
            w.load_more_search(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.search_results.len(), 450);
            assert!(!w.ui.search_has_more);
            w.search
                .update(cx, |s, cx| s.set_value("missing", window, cx));
            w.run_search(cx);
            assert_eq!(w.ui.search_limit, 200);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert!(w.search_results.is_empty());
            assert!(!w.ui.search_loading);
        })
        .unwrap();
}
#[gpui::test]
fn grouped_search_collapses_rows_and_header_opens_first_match(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let source = "前言😀 alpha\n下一行 alpha\n";
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some(source.into()), false, window, cx);
            Arc::make_mut(&mut w.index).update("a.md".into(), source.into());
            Arc::make_mut(&mut w.index).update("b.md".into(), "alpha".into());
            w.fulltext = true;
            w.ui.left_mode = 1;
            w.search
                .update(cx, |s, cx| s.set_value("alpha", window, cx));
            w.run_search(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1100.), px(650.)));
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let expanded = handle
        .update(&mut visual, |w, _, cx| {
            assert_eq!(w.search_results.len(), 3);
            assert!(w.ui.search_group_scroll.bounds_for_item(2).is_none());
            let height =
                w.ui.search_group_scroll
                    .bounds_for_item(0)
                    .unwrap()
                    .size
                    .height;
            w.ui.search_collapsed.insert("a.md".into());
            cx.notify();
            height
        })
        .unwrap();
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let click = handle
        .update(&mut visual, |w, _, _| {
            let bounds = w.ui.search_group_scroll.bounds_for_item(0).unwrap();
            assert!(bounds.size.height < expanded);
            bounds.origin + point(px(70.), px(14.))
        })
        .unwrap();
    visual.simulate_click(click, Modifiers::default());
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(
                w.current_pane().unwrap().read(cx).editor.read(cx).cursor(),
                source.find("alpha").unwrap()
            );
            w.search
                .update(cx, |s, cx| s.set_value("changed", window, cx));
            w.run_search(cx);
            assert!(w.ui.search_collapsed.is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn fulltext_case_setting_changes_background_results(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            Arc::make_mut(&mut w.index).update("note.md".into(), "Alpha\nalpha".into());
            w.fulltext = true;
            w.search
                .update(cx, |s, cx| s.set_value("Alpha", window, cx));
            w.run_search(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.search_results.len(), 2);
            w.ui.prefs.search_case_sensitive = true;
            w.run_search(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert_eq!(w.search_results.len(), 1);
            assert_eq!(w.search_results[0].line, 1);
        })
        .unwrap();
}
#[gpui::test]
fn quick_switch_queries_do_not_replace_fulltext_search(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.focus_search(true, window, cx);
            w.search
                .update(cx, |s, cx| s.set_value("tag:work file:note", window, cx));
            w.run_search(cx);
            w.focus_search(false, window, cx);
            assert!(w.search.read(cx).value().is_empty());
            w.search
                .update(cx, |s, cx| s.set_value("filename query", window, cx));
            w.run_search(cx);
            w.close_overlays(window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "tag:work file:note");
            assert!(w.fulltext);
            w.focus_search(false, window, cx);
            assert_eq!(w.ui.prefs.search_query, "tag:work file:note");
            w.focus_search(false, window, cx);
            w.add_tab("note.md".into(), Some("body".into()), false, window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "tag:work file:note");
        })
        .unwrap();
}
#[gpui::test]
fn tag_search_combines_fulltext_filters_but_not_quick_switch_queries(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.fulltext = true;
            w.search
                .update(cx, |s, cx| s.set_value("file:note", window, cx));
            w.search_tag("work", true, window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "file:note tag:work");
            w.search_tag("work", true, window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "file:note");
            w.fulltext = false;
            w.search
                .update(cx, |s, cx| s.set_value("temporary filename", window, cx));
            w.search_tag("home", true, window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "file:note tag:home");
            assert!(w.fulltext);
        })
        .unwrap();
}
#[gpui::test]
fn tag_navigation_folds_and_opens_selected_search(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            Arc::make_mut(&mut w.index).update("note.md".into(), "#work/one #work/two".into());
            w.ui.right_mode = 3;
            window.focus(&w.ui.tags_focus, cx);
            assert!(w.navigate_tags("down", window, cx));
            assert_eq!(w.ui.tags_selected.as_deref(), Some("work"));
            w.navigate_tags("left", window, cx);
            assert!(w.ui.prefs.tags.collapsed.contains("work"));
            w.navigate_tags("right", window, cx);
            assert!(!w.ui.prefs.tags.collapsed.contains("work"));
            w.navigate_tags("right", window, cx);
            assert_eq!(w.ui.tags_selected.as_deref(), Some("work/one"));
            w.navigate_tags("enter", window, cx);
            assert_eq!(w.search.read(cx).value().as_ref(), "tag:work/one");
            assert!(w.fulltext);
            assert_eq!(w.ui.left_mode, 1);
        })
        .unwrap();
}
#[gpui::test]
fn property_list_conversion_preserves_scalar_commas_and_pending_items(cx: &mut TestAppContext) {
    use inkstone_core::properties::Kind;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab(
                "note.md".into(),
                Some("---\naliases: 'Smith, John'\n---\nbody".into()),
                false,
                window,
                cx,
            );
            w.edit_property("aliases", "\"Smith, John\"", window, cx);
            assert_eq!(w.property_list_values(cx).unwrap(), ["Smith, John"]);
            w.save_property(window, cx);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            assert_eq!(
                inkstone_core::index::parse(&editor.read(cx).value()).aliases,
                ["Smith, John"]
            );
            w.edit_property("", "", window, cx);
            w.ui.property_key
                .update(cx, |s, cx| s.set_value("custom", window, cx));
            w.ui.property_value
                .update(cx, |s, cx| s.set_value("One, two", window, cx));
            w.change_property_kind(Kind::List, window, cx);
            assert_eq!(w.property_list_values(cx).unwrap(), ["One, two"]);
            w.ui.property_list_entry
                .update(cx, |s, cx| s.set_value("未确认的项目", window, cx));
            w.change_property_kind(Kind::Text, window, cx);
            assert!(
                w.ui.property_value
                    .read(cx)
                    .value()
                    .contains("未确认的项目")
            );
            assert!(w.ui.property_list_entry.read(cx).value().is_empty());
            w.close_overlays(window, cx);
            assert!(!editor.read(cx).value().contains("custom:"));
        })
        .unwrap();
}
#[gpui::test]
fn property_list_items_add_remove_save_and_undo_without_losing_punctuation(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let source = "---\naliases: [原始]\n---\n正文";
            w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
            w.edit_property("aliases", "[\"原始\"]", window, cx);
            w.ui.property_list_entry
                .update(cx, |s, cx| s.set_value("Smith, John", window, cx));
            assert!(w.add_property_list_item(window, cx));
            assert_eq!(w.property_list_values(cx).unwrap(), ["原始", "Smith, John"]);
            w.remove_property_list_item(0, window, cx);
            w.ui.property_list_entry
                .update(cx, |s, cx| s.set_value("含\"引号\"的项目", window, cx));
            w.save_property(window, cx);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            assert_eq!(
                inkstone_core::index::parse(&editor.read(cx).value()).aliases,
                ["Smith, John", "含\"引号\"的项目"]
            );
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), source);
            w.edit_property("aliases", "[\"原始\"]", window, cx);
            w.remove_property_list_item(0, window, cx);
            w.close_overlays(window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn property_calendar_syncs_manual_values_and_saves_selected_dates(cx: &mut TestAppContext) {
    use gpui_component::date_picker::{DatePickerEvent, DateTime};
    use inkstone_core::properties::Kind;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab(
                "note.md".into(),
                Some("---\ndate: '2024-02-29'\n---\nbody".into()),
                false,
                window,
                cx,
            );
            w.ui.prefs.property_types.insert("date".into(), Kind::Date);
            w.edit_property("date", "\"2024-02-29\"", window, cx);
            assert_eq!(
                w.ui.property_dates[0]
                    .read(cx)
                    .date_time()
                    .start()
                    .unwrap()
                    .date(),
                chrono::NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
            );
            let selected = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
                .unwrap()
                .and_hms_opt(12, 34, 56)
                .unwrap();
            w.ui.property_dates[0].update(cx, |_, cx| {
                cx.emit(DatePickerEvent::Change(DateTime::Single(Some(selected))))
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.ui.property_value.read(cx).value().as_ref(), "2026-10-01");
            w.save_property(window, cx);
            assert!(
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("date: \"2026-10-01\"")
            );
            w.ui.prefs
                .property_types
                .insert("date".into(), Kind::DateTime);
            w.edit_property("date", "\"2026-10-01T12:34:56\"", window, cx);
            assert_eq!(
                w.ui.property_dates[1]
                    .read(cx)
                    .date_time()
                    .start()
                    .unwrap()
                    .format("%H:%M:%S")
                    .to_string(),
                "12:34:56"
            );
            w.accept_property_date(1, DateTime::Single(None), window, cx);
            assert!(w.ui.property_value.read(cx).value().is_empty());
            w.close_overlays(window, cx);
            w.accept_property_date(
                1,
                DateTime::Single(Some(chrono::Local::now().naive_local())),
                window,
                cx,
            );
            assert!(w.ui.property_value.read(cx).value().is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn property_types_preserve_numeric_text_and_validate_conversions(cx: &mut TestAppContext) {
    use inkstone_core::properties::Kind;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let source = "---\ncode: '123'\n---\nbody";
            w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
            w.edit_property("code", "\"123\"", window, cx);
            w.save_property(window, cx);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            assert!(editor.read(cx).value().contains("code: \"123\""));
            assert_eq!(w.ui.prefs.property_types["code"], Kind::Text);
            w.edit_property("code", "\"123\"", window, cx);
            w.ui.property_kind = Kind::Number;
            w.save_property(window, cx);
            assert!(editor.read(cx).value().contains("code: 123"));
            assert_eq!(w.ui.prefs.property_types["code"], Kind::Number);
            w.edit_property("code", "123", window, cx);
            w.ui.property_value
                .update(cx, |s, cx| s.set_value("not a number", window, cx));
            w.save_property(window, cx);
            assert!(w.ui.property_open);
            assert!(editor.read(cx).value().contains("code: 123"));
            assert_eq!(w.ui.property_error, "请输入有效数字。");
            w.edit_property("code", "123", window, cx);
            assert!(w.ui.property_error.is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn property_rename_delete_undo_and_stale_dialog(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let source = "---\nold: value\nother: keep\n---\n正文";
            w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            w.edit_property("old", "value", window, cx);
            w.ui.property_key
                .update(cx, |s, cx| s.set_value("new", window, cx));
            w.save_property(window, cx);
            assert!(!editor.read(cx).value().contains("old:"));
            assert!(editor.read(cx).value().contains("new:"));
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), source);
            w.edit_property("old", "value", window, cx);
            w.delete_property(window, cx);
            assert_eq!(
                editor.read(cx).value().as_ref(),
                "---\nother: keep\n---\n正文"
            );
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), source);
            w.edit_property("old", "value", window, cx);
            editor.update(cx, |s, cx| {
                s.replace_all(format!("{source}外部变化"), window, cx)
            });
            w.delete_property(window, cx);
            assert!(w.ui.property_open);
            assert_eq!(
                editor.read(cx).value().as_ref(),
                format!("{source}外部变化")
            );
            assert!(w.status.contains("笔记已变更"));
            assert_eq!(w.ui.property_error, w.status);
        })
        .unwrap();
}
#[gpui::test]
fn property_editor_preserves_multiline_aliases_and_rejects_invalid_lists(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let source = "---\naliases:\n  - 'Smith, John'\n  - 中文\n---\n正文😀";
            w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
            let property = inkstone_core::properties::parse(source).remove(0);
            w.edit_property(&property.name, &property.value, window, cx);
            assert!(w.ui.property_value.read(cx).value().contains("Smith, John"));
            w.save_property(window, cx);
            assert!(!w.ui.property_open);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            assert_eq!(
                inkstone_core::index::parse(&editor.read(cx).value()).aliases,
                ["Smith, John", "中文"]
            );
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), source);
            w.edit_property("aliases", "[broken", window, cx);
            w.save_property(window, cx);
            assert!(w.ui.property_open);
            assert_eq!(editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn reading_quote_link_opens_its_source_relative_note(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-reading-link-{}", std::process::id()));
    std::fs::create_dir_all(root.join("folder")).unwrap();
    let source = "> [前往](../other.md)\n";
    std::fs::write(root.join("folder/root.md"), source).unwrap();
    std::fs::write(root.join("other.md"), "# 目的地").unwrap();
    handle
        .update(cx, |w, window, cx| {
            let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
            w.index = Arc::new(Index::build(&vault).unwrap());
            w.vault = Some(vault);
            w.add_tab(
                "folder/root.md".into(),
                Some(source.into()),
                false,
                window,
                cx,
            );
            w.tabs[0].pane.update(cx, |p, cx| {
                p.reading = true;
                p.focus_view(window, cx);
                cx.notify();
            });
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(820.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let bounds = handle
        .update(&mut visual, |w, _, cx| {
            w.tabs[0].pane.read(cx).reading_bounds(cx)
        })
        .unwrap();
    visual.simulate_click(
        bounds.origin + point(px(28.), px(10.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    handle
        .update(&mut visual, |w, _, _| {
            assert_eq!(
                w.tabs[w.active.unwrap()].path,
                PathBuf::from("other.md"),
                "bounds={bounds:?}; status={}",
                w.status
            )
        })
        .unwrap();
    visual.run_until_parked();
    std::fs::remove_dir_all(root).unwrap();
}
#[gpui::test]
fn file_location_settings_control_note_and_attachment_writes(cx: &mut TestAppContext) {
    use inkstone_core::locations::Location;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-file-locations-{}", std::process::id()));
    std::fs::create_dir_all(root.join("project")).unwrap();
    std::fs::write(root.join("project/start.md"), "").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab(
                "project/start.md".into(),
                Some("".into()),
                false,
                window,
                cx,
            );
            w.ui.prefs.locations.notes = Location::Current;
            w.focus_new(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(root.join("project/未命名.md").is_file());
    handle
        .update(cx, |w, window, cx| {
            w.ui.prefs.locations.attachments = Location::Subfolder;
            w.ui.prefs.locations.attachment_folder = "media".into();
            w.paste_image("image.png".into(), b"fixture".to_vec(), window, cx);
            w.current_pane()
                .unwrap()
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
                });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.ui.pending_file_writes, 1);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            assert_eq!(editor.read(cx).value(), "ni");
            editor.update(cx, |s, cx| s.replace_text_in_range(None, "你", window, cx));
        })
        .unwrap();
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    assert!(!root.join("project/media/skip.png").exists());
    handle
        .update(cx, |w, window, cx| {
            let text = w.tabs[w.active.unwrap()]
                .pane
                .read(cx)
                .editor
                .read(cx)
                .value();
            assert!(text.contains("![[image.png]]"));
            w.save_all(window, cx);
            w.ui.prefs.locations.notes = Location::Folder;
            w.ui.prefs.locations.note_folder = "inbox".into();
            w.focus_new(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(root.join("inbox/未命名.md").is_file());
    assert_eq!(
        std::fs::read(root.join("project/media/image.png")).unwrap(),
        b"fixture"
    );
    handle
        .update(cx, |w, window, cx| {
            let count = w.tabs.len();
            w.ui.prefs.locations.note_folder = "../outside".into();
            w.focus_new(window, cx);
            assert_eq!(w.tabs.len(), count);
            assert!(w.status.contains("库内文件夹"));
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn duplicate_opens_snapshot_without_retargeting_original_tab(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-duplicate-ui-{}", std::process::id()));
    std::fs::create_dir_all(root.join("folder")).unwrap();
    std::fs::write(root.join("folder/source.md"), "original").unwrap();
    let original = handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab(
                "folder/source.md".into(),
                Some("original".into()),
                false,
                window,
                cx,
            );
            let id = w.tabs[0].id;
            w.tabs[0].pane.read(cx).editor.clone().update(cx, |s, cx| {
                s.set_selected_range(8..8, cx);
                s.replace(" 新编辑😀", window, cx);
            });
            w.duplicate_current(window, cx);
            assert_eq!(w.ui.pending_file_writes, 1);
            id
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.ui.pending_file_writes, 0);
            assert_eq!(w.tabs.len(), 2);
            let old = w.tabs.iter().find(|t| t.id == original).unwrap();
            assert_eq!(old.path, PathBuf::from("folder/source.md"));
            assert_eq!(old.save.persistence.baseline().as_deref(), Some("original"));
            assert_eq!(
                old.pane.read(cx).editor.read(cx).value(),
                "original 新编辑😀"
            );
            let copy = &w.tabs[w.active.unwrap()];
            assert_eq!(copy.path, PathBuf::from("folder/source 副本.md"));
            assert_eq!(
                copy.save.persistence.baseline().as_deref(),
                Some("original 新编辑😀")
            );
            assert_eq!(
                std::fs::read_to_string(root.join(&copy.path)).unwrap(),
                "original 新编辑😀"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("folder/source.md")).unwrap(),
                "original"
            );
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn explorer_sort_preserves_folders_selection_and_reacts_to_dates(cx: &mut TestAppContext) {
    use inkstone_core::file_order::SortBy;
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, _, cx| {
            let mut index = Index::default();
            index.update("note10.md".into(), "ten".into());
            index.update("note2.md".into(), "two".into());
            let old = std::time::SystemTime::UNIX_EPOCH;
            Arc::make_mut(
                index
                    .notes
                    .get_mut(std::path::Path::new("note10.md"))
                    .unwrap(),
            )
            .times
            .modified = Some(old);
            Arc::make_mut(
                index
                    .notes
                    .get_mut(std::path::Path::new("note2.md"))
                    .unwrap(),
            )
            .times
            .modified = Some(old + Duration::from_secs(10));
            w.index = Arc::new(index);
            w.ui.folders = vec!["Folder".into()];
            w.sync_index_ui(cx);
            assert!(w.tree.read(cx).entry(0).unwrap().item().is_folder());
            assert_eq!(
                w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                "note2.md"
            );
            w.tree
                .update(cx, |tree, cx| tree.set_selected_index(Some(2), cx));
            w.ui.prefs.sort_descending = true;
            w.rebuild_sorted_tree(cx);
            assert!(w.tree.read(cx).entry(0).unwrap().item().is_folder());
            assert_eq!(
                w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                "note10.md"
            );
            assert_eq!(
                w.tree.read(cx).selected_item().unwrap().label.as_ref(),
                "note10.md"
            );
            w.ui.prefs.sort_by = SortBy::Modified;
            w.sync_index_ui(cx);
            assert_eq!(
                w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                "note2.md"
            );
            Arc::make_mut(
                Arc::make_mut(&mut w.index)
                    .notes
                    .get_mut(std::path::Path::new("note10.md"))
                    .unwrap(),
            )
            .times
            .modified = Some(old + Duration::from_secs(20));
            w.sync_index_ui(cx);
            assert_eq!(
                w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                "note10.md"
            );
            assert_eq!(
                w.tree.read(cx).selected_item().unwrap().label.as_ref(),
                "note10.md"
            );
        })
        .unwrap();
}

#[gpui::test]
fn inline_title_renames_original_tab_and_preserves_failed_input(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-inline-title-{}", std::process::id()));
    std::fs::create_dir_all(root.join("folder")).unwrap();
    std::fs::write(root.join("folder/old.md"), "body").unwrap();
    std::fs::write(root.join("folder/other.md"), "[[old]]").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.ui.prefs.always_update_links = true;
            w.add_tab(
                "folder/old.md".into(),
                Some("body".into()),
                false,
                window,
                cx,
            );
            w.add_tab(
                "folder/other.md".into(),
                Some("[[old]]".into()),
                false,
                window,
                cx,
            );
            w.begin_inline_title(0, false, window, cx);
        })
        .unwrap();
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    handle
        .update(cx, |w, window, cx| {
            let input = w.ui.inline_title.as_ref().unwrap().input.clone();
            input.update(cx, |s, cx| s.set_value("新名😀", window, cx));
            w.focus_primary(1, window, cx);
        })
        .unwrap();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(
                root.join("folder/新名😀.md").is_file(),
                "status={}, title={:?}, pending={}",
                w.status,
                w.ui.inline_title
                    .as_ref()
                    .map(|edit| edit.input.read(cx).value()),
                w.ui.pending_file_writes
            );
            assert!(!root.join("folder/old.md").exists());
            assert_eq!(
                w.tabs[w.active.unwrap()].path,
                PathBuf::from("folder/other.md")
            );
            assert!(
                std::fs::read_to_string(root.join("folder/other.md"))
                    .unwrap()
                    .contains("新名😀")
            );
            assert!(w.ui.inline_title.is_none());
            w.begin_inline_title(0, false, window, cx);
            let input = w.ui.inline_title.as_ref().unwrap().input.clone();
            input.update(cx, |s, cx| s.set_value("../bad", window, cx));
            w.commit_inline_title(true, window, cx);
            assert!(!w.ui.file_operation);
            assert!(w.ui.inline_title.is_some());
            input.update(cx, |s, cx| s.set_value("other", window, cx));
            w.commit_inline_title(true, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.ui.inline_title
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .value()
                    .as_ref(),
                "other"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("folder/新名😀.md")).unwrap(),
                "body"
            );
            w.cancel_inline_title(window, cx);
            assert!(w.ui.inline_title.is_none());
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn rename_link_prompt_supports_skip_once_always_and_conflict(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-link-prompt-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "A").unwrap();
    std::fs::write(root.join("source.md"), "[[a]]").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
            w.name.update(cx, |s, cx| s.set_value("b.md", window, cx));
            w.manage_note(false, window, cx);
            w.ui.property_open = true;
            w.ui.property_value
                .update(cx, |s, cx| s.set_value("未提交值", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(root.join("b.md").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("source.md")).unwrap(),
        "[[a]]"
    );
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.ui.link_update.as_ref().unwrap().len(), 1);
            w.close_overlays(window, cx);
            assert!(w.ui.property_open);
            assert_eq!(w.ui.property_value.read(cx).value(), "未提交值");
            w.close_overlays(window, cx);
        })
        .unwrap();
    std::fs::write(root.join("source.md"), "[[b]]").unwrap();
    for (name, always) in [("c.md", false), ("d.md", true)] {
        handle
            .update(cx, |w, window, cx| {
                w.name.update(cx, |s, cx| s.set_value(name, window, cx));
                w.manage_note(false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.link_update.is_some());
                w.confirm_link_updates(always, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            format!("[[/{}]]", name.trim_end_matches(".md"))
        );
    }
    handle
        .update(cx, |w, window, cx| {
            assert!(w.ui.prefs.always_update_links);
            w.name.update(cx, |s, cx| s.set_value("e.md", window, cx));
            w.manage_note(false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("source.md")).unwrap(),
        "[[/e]]"
    );
    handle
        .update(cx, |w, window, cx| {
            assert!(w.ui.link_update.is_none());
            w.ui.prefs.always_update_links = false;
            w.name.update(cx, |s, cx| s.set_value("f.md", window, cx));
            w.manage_note(false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::write(root.join("source.md"), "外部变更").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.confirm_link_updates(false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("source.md")).unwrap(),
        "外部变更"
    );
    handle
        .update(cx, |w, _, _| {
            assert!(w.status.contains("未更新"));
            assert!(!w.ui.file_operation);
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn folder_move_updates_disk_open_tabs_and_session_paths(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-folder-move-ui-{}", std::process::id()));
    std::fs::create_dir_all(root.join("old")).unwrap();
    let source = "[[target]] [outside](../outside.md)";
    std::fs::write(root.join("old/note.md"), source).unwrap();
    std::fs::write(root.join("old/target.md"), "target").unwrap();
    std::fs::write(root.join("outside.md"), "[[old/note|alias]]").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.ui.prefs.always_update_links = true;
            let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
            w.index = Arc::new(Index::build(&vault).unwrap());
            w.vault = Some(vault);
            w.add_tab("old/note.md".into(), Some(source.into()), false, window, cx);
            w.ui.prefs.bookmarks.push("old/note.md".into());
            w.ui.closed.push("old/target.md".into());
            w.manage_folder("old".into(), Some("archive/new".into()), window, cx);
            assert!(w.ui.file_operation);
            assert_eq!(w.ui.pending_file_writes, 1);
            w.save_all(window, cx);
            assert!(!w.tabs[0].save.persistence.is_saving());
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert!(!w.ui.file_operation);
            assert_eq!(w.ui.pending_file_writes, 0);
            assert_eq!(w.tabs[0].path, PathBuf::from("archive/new/note.md"));
            assert_eq!(
                w.ui.prefs.bookmarks[0],
                PathBuf::from("archive/new/note.md")
            );
            assert_eq!(w.ui.closed[0], PathBuf::from("archive/new/target.md"));
            let disk = std::fs::read_to_string(root.join("archive/new/note.md")).unwrap();
            assert_eq!(disk, "[[/archive/new/target]] [outside](../../outside.md)");
            assert_eq!(w.tabs[0].save.persistence.baseline().as_ref(), Some(&disk));
            assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), disk);
            assert!(!w.tabs[0].save.persistence.has_conflict());
            assert_eq!(
                std::fs::read_to_string(root.join("outside.md")).unwrap(),
                "[[/archive/new/note|alias]]"
            );
        })
        .unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.manage_folder("archive/new".into(), Some("final".into()), window, cx);
            let editor = w.tabs[0].pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                let end = s.value().len();
                s.set_selected_range(end..end, cx);
                s.replace(" 新编辑😀", window, cx);
            });
            w.tabs[0].save.persistence.test_set_dirty(true);
            w.save_pending(window, cx);
            assert!(!w.tabs[0].save.persistence.is_saving());
            w.refresh_requested = true;
            w.refresh(window, cx);
            assert!(w.refresh_requested);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.tabs[0].path, PathBuf::from("final/note.md"));
            assert!(w.tabs[0].save.persistence.has_conflict());
            assert!(
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .ends_with(" 新编辑😀")
            );
            assert!(!root.join("archive/new").exists());
            let disk = std::fs::read_to_string(root.join("final/note.md")).unwrap();
            assert_eq!(disk, "[[/final/target]] [outside](../outside.md)");
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn trashing_a_folder_updates_the_tree_without_rereading_other_notes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!(
        "inkstone-folder-trash-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("资料")).unwrap();
    std::fs::write(root.join("keep.md"), "# Keep").unwrap();
    std::fs::write(root.join("资料/note.md"), "# Moved").unwrap();
    std::fs::write(root.join("资料/image.png"), []).unwrap();
    let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
    let index = Arc::new(Index::build(&vault).unwrap());
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(vault);
            w.index = index.clone();
            w.files = index.note_paths();
            w.ui.folders = vec!["资料".into()];
            w.sync_index_ui(cx);
            w.add_tab(
                "资料/note.md".into(),
                Some("# Moved".into()),
                false,
                window,
                cx,
            );
            w.ui.prefs.bookmarks.push("资料/note.md".into());
            w.ui.prefs.expanded_folders.push("资料".into());
            // A full rebuild would fail while decoding this unchanged note.
            std::fs::write(root.join("keep.md"), [0xff]).unwrap();
            w.manage_folder("资料".into(), None, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.status, "文件夹已移入可恢复回收站。");
            assert!(!w.ui.file_operation);
            assert!(
                w.tabs
                    .iter()
                    .all(|tab| tab.path != std::path::Path::new("资料/note.md"))
            );
            assert!(w.ui.prefs.bookmarks.is_empty());
            assert!(w.ui.prefs.expanded_folders.is_empty());
            assert!(!w.ui.folders.iter().any(|path| path.starts_with("资料")));
            assert!(
                !w.index
                    .notes
                    .contains_key(std::path::Path::new("资料/note.md"))
            );
            assert!(std::sync::Arc::ptr_eq(
                &index.notes[std::path::Path::new("keep.md")],
                &w.index.notes[std::path::Path::new("keep.md")]
            ));
            assert!(!w.index.files.contains(&PathBuf::from("资料/image.png")));
            let tree = w.tree.read(cx);
            assert!(tree.index_of(&gpui::SharedString::from("资料")).is_none());
            assert!(
                tree.index_of(&gpui::SharedString::from("keep.md"))
                    .is_some()
            );
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn embedded_task_write_checks_the_source_snapshot(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-embedded-task-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = "- [ ] 源任务😀\r\n";
    std::fs::write(root.join("source.md"), source).unwrap();
    let marker = inkstone_core::index::parse(source).tasks[0].marker.clone();
    let target = inkstone_core::rendering::TaskTarget {
        rendered_start: 0,
        path: "source.md".into(),
        marker,
        baseline: std::sync::Arc::from(source),
    };
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.toggle_referenced_task(target.clone(), true, window, cx);
            assert_eq!(w.ui.pending_file_writes, 1);
            assert!(!w.request_window_close(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("source.md")).unwrap(),
        "- [x] 源任务😀\r\n"
    );
    handle
        .update(cx, |w, _, _| {
            assert_eq!(w.ui.pending_file_writes, 0);
            assert!(
                w.index
                    .notes
                    .get(std::path::Path::new("source.md"))
                    .unwrap()
                    .parsed
                    .tasks[0]
                    .checked
            );
        })
        .unwrap();
    std::fs::write(root.join("source.md"), "外部已更新").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.toggle_referenced_task(target, false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("source.md")).unwrap(),
        "外部已更新"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn menu_quit_waits_for_saves_and_blocks_conflicts(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-menu-quit-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("退出保存.md".into(), None, true, window, cx);
            w.tabs[0].pane.read(cx).editor.clone().update(cx, |s, cx| {
                s.replace("菜单退出前保存", window, cx);
            });
            w.tabs[0].save.persistence.test_set_conflict(true);
            w.request_app_quit(window, cx);
            assert!(!w.quit_requested);
            assert!(!w.ui.window_close_requested);
            w.tabs[0].save.persistence.test_set_conflict(false);
            w.request_app_quit(window, cx);
            assert!(w.quit_requested);
            assert!(w.ui.window_close_requested);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.request_window_close(window, cx));
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("退出保存.md")).unwrap(),
        "菜单退出前保存"
    );
    let prefs =
        inkstone_core::preferences::Preferences::load(&root.join(".inkstone-workspace.json"));
    assert_eq!(prefs.active_path, Some("退出保存.md".into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn failed_workspace_save_can_retry_or_close_without_bypassing_note_safety(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root =
        std::env::temp_dir().join(format!("inkstone-settings-failure-{}", std::process::id()));
    std::fs::create_dir_all(root.join(".inkstone-workspace.json")).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("safe.md".into(), Some("original".into()), false, window, cx);
            assert!(!w.request_window_close(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.ui.persist_error.is_some());
            assert!(!w.ui.window_close_requested);
            w.tick(window, cx);
            assert!(!w.ui.persisting, "failed saves do not retry forever");
            assert!(!w.request_window_close(window, cx));
            w.ui.discard_workspace_on_close = true;
            assert!(w.request_window_close(window, cx));
            w.tabs[0].save.persistence.test_set_conflict(true);
            assert!(
                !w.request_window_close(window, cx),
                "discarding layout never discards notes"
            );
            w.tabs[0].save.persistence.test_set_conflict(false);
            w.ui.discard_workspace_on_close = false;
            w.ui.persist_error = None;
            std::fs::remove_dir(root.join(".inkstone-workspace.json")).unwrap();
            w.persist_workspace(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.ui.persist_error.is_none());
            assert!(w.request_window_close(window, cx));
        })
        .unwrap();
    assert!(root.join(".inkstone-workspace.json").is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn window_close_waits_for_note_and_session_writes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-shutdown-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("关闭保存.md".into(), None, true, window, cx);
            w.tabs[0]
                .pane
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.replace("关闭时的内容😀", window, cx));
            assert!(!w.request_window_close(window, cx));
            assert!(w.ui.window_close_requested);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.request_window_close(window, cx));
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("关闭保存.md")).unwrap(),
        "关闭时的内容😀"
    );
    let prefs =
        inkstone_core::preferences::Preferences::load(&root.join(".inkstone-workspace.json"));
    assert_eq!(prefs.active_path, Some("关闭保存.md".into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn split_views_share_edits_and_undo_but_keep_independent_focus(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab("a.md".into(), Some("中文😀".into()), false, window, cx);
            w.split_active(false, window, cx);
            let editor = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| {
                s.set_selected_range("中文😀".len().."中文😀".len(), cx);
                s.replace(" 新文本", window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.views.secondary_focused);
            assert!(w.tabs[0].save.persistence.is_dirty());
            let canonical = w.tabs[0].pane.read(cx).editor.clone();
            assert_eq!(canonical.read(cx).value().as_ref(), "中文😀 新文本");
            assert_eq!(canonical.read(cx).selected_range(), 0..0);
            canonical.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            let mirror = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
            assert_eq!(mirror.read(cx).value().as_ref(), "中文😀");
            assert!(!w.tabs[0].save.persistence.is_dirty());
            w.tabs[0]
                .pane
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.redo(&gpui_component::input::Redo, window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .as_ref(),
                "中文😀 新文本"
            );
            w.add_tab("b.md".into(), Some("B".into()), false, window, cx);
            assert_eq!(w.tabs[w.main_tab().unwrap()].path, PathBuf::from("a.md"));
            assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("b.md"));
            assert_ne!(w.tabs[1].pane, w.current_pane().unwrap());
            w.close_split(window, cx);
            assert!(w.views.split.is_none());
            assert_eq!(w.active, Some(0));
            assert_eq!(w.tabs.len(), 2);
        })
        .unwrap();
}

#[gpui::test]
fn quick_switcher_results_stay_inside_short_window_and_scroll_to_selection(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.files = (0..30)
                .map(|i| PathBuf::from(format!("folder/中文笔记-{i:02}.md")))
                .collect();
            w.focus_search(false, window, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(500.)));
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, _, _| {
            let bounds = w.ui.modal_scroll.bounds();
            assert!(
                bounds.size.height > px(0.) && bounds.size.height <= px(360.),
                "{bounds:?}"
            );
            assert!(bounds.bottom() <= px(484.), "{bounds:?}");
        })
        .unwrap();
    visual.simulate_platform_keystrokes("down down down down down down down down down down down down down down down down down down down down");
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, _, _| {
            assert_eq!(w.ui.selected, 20);
            let bounds = w.ui.modal_scroll.bounds();
            let item = w.ui.modal_scroll.bounds_for_item(20).unwrap();
            let offset = w.ui.modal_scroll.offset();
            assert!(offset.y < px(0.));
            assert!(item.top() + offset.y >= bounds.top() - px(1.));
            assert!(item.bottom() + offset.y <= bounds.bottom() + px(1.));
        })
        .unwrap();
}

#[gpui::test]
fn reopen_closed_tab_restores_mode_selection_and_folds(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-reopen-view-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let text = "# Heading\nbody\nmore\n\n# Second\ntail";
    std::fs::write(root.join("note.md"), text).unwrap();
    let handle = cx.add_window(Workspace::new);
    let expected = handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("note.md".into(), Some(text.into()), false, window, cx);
            let pane = w.current_pane().unwrap();
            pane.update(cx, |p, cx| {
                p.fold_sections(Some(true), window, cx);
                p.reading = true;
                p.live = false;
                p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
            });
            let state = Workspace::snapshot_view("note.md".into(), &pane, cx);
            assert!(!state.folded_lines.is_empty());
            w.close_tab_at(0, window, cx);
            assert!(w.tabs.is_empty());
            w.execute_command(17, window, cx);
            state
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.tabs.len(), 1);
            let pane = w.current_pane().unwrap();
            let actual = Workspace::snapshot_view("note.md".into(), &pane, cx);
            assert!(actual.reading);
            assert!(!actual.live);
            assert_eq!(actual.selection, expected.selection);
            assert_eq!(actual.folded_lines, expected.folded_lines);
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn middle_click_closes_only_the_pressed_tab_and_preserves_conflicts(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            for name in ["a", "b", "c"] {
                w.add_tab(
                    format!("{name}.md").into(),
                    Some(name.into()),
                    false,
                    window,
                    cx,
                );
            }
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let position = handle
        .update(&mut visual, |w, _, _| {
            w.ui.tab_scroll.bounds_for_item(0).unwrap().center() + w.ui.tab_scroll.offset()
        })
        .unwrap();
    visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
    handle
        .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 3))
        .unwrap();
    visual.simulate_mouse_down(position, MouseButton::Middle, Modifiers::default());
    visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
    handle
        .update(&mut visual, |w, _, _| {
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("c.md"));
            w.tabs[0].save.persistence.test_set_conflict(true);
        })
        .unwrap();
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let position = handle
        .update(&mut visual, |w, _, _| {
            w.ui.tab_scroll.bounds_for_item(0).unwrap().center() + w.ui.tab_scroll.offset()
        })
        .unwrap();
    visual.simulate_mouse_down(position, MouseButton::Middle, Modifiers::default());
    visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
    handle
        .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 2))
        .unwrap();
}

#[gpui::test]
fn closing_background_tab_preserves_current_editor(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            for name in ["a", "b", "c"] {
                w.add_tab(
                    format!("{name}.md").into(),
                    Some(name.into()),
                    false,
                    window,
                    cx,
                );
            }
            let active = w.tabs[2].pane.clone();
            w.close_tab_at(0, window, cx);
            assert_eq!(w.tabs[w.active.unwrap()].pane, active);
            assert!(
                active
                    .read(cx)
                    .editor
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert_eq!(w.ui.closed, vec![PathBuf::from("a.md")]);
        })
        .unwrap();
}
#[gpui::test]
fn session_restores_interleaved_blank_tabs_and_active_blank(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-blank-session-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "A").unwrap();
    std::fs::write(root.join("b.md"), "B").unwrap();
    inkstone_core::preferences::Preferences {
        open_paths: ["missing.md", "", "a.md", "", "b.md", ""]
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        active_path: Some(PathBuf::new()),
        active_tab_index: Some(3),
        ..Default::default()
    }
    .save(&root.join(".inkstone-workspace.json"))
    .unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.tabs.iter().map(|t| t.path.clone()).collect::<Vec<_>>(),
                ["", "a.md", "", "b.md", ""]
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
            );
            assert_eq!(w.active, Some(2));
            w.add_tab("c.md".into(), Some("C".into()), false, window, cx);
            assert_eq!(w.tabs.len(), 5);
            assert_eq!(
                w.tabs
                    .iter()
                    .filter(|t| t.path.as_os_str().is_empty())
                    .count(),
                2
            );
            w.watcher = None;
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn quick_capture_respects_location_and_opens_distinct_editable_notes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-capture-{}", std::process::id()));
    std::fs::create_dir_all(root.join("收集")).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.ui.prefs.locations.notes = inkstone_core::locations::Location::Folder;
            w.ui.prefs.locations.note_folder = "收集".into();
            w.ui.prefs.default_reading = true;
            w.quick_capture(window, cx);
            w.quick_capture(window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert_ne!(w.tabs[0].path, w.tabs[1].path);
            assert!(
                w.tabs
                    .iter()
                    .all(|t| t.path.starts_with("收集") && !t.pane.read(cx).reading)
            );
            w.tabs[1]
                .save
                .editor
                .update(cx, |s, cx| s.set_value("中文 👩‍💻 速记", window, cx));
            w.save_all(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| w.save_all(window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert_eq!(
                std::fs::read_to_string(root.join(&w.tabs[1].path)).unwrap(),
                "中文 👩‍💻 速记"
            );
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn empty_tabs_reuse_their_slot_and_new_notes_receive_unique_names(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-blank-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.new_blank(window, cx);
            assert!(w.tabs[0].path.as_os_str().is_empty());
            assert!(!w.tabs[0].save.persistence.is_dirty());
            w.focus_new(window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.tabs[0].path, PathBuf::from("未命名.md"));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            w.new_blank(window, cx);
            assert_eq!(w.tabs.len(), 2);
            w.open_note("未命名.md".into(), window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[1].path, PathBuf::from("未命名.md"));
            assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
            w.focus_new(window, cx);
            assert_eq!(w.tabs[2].path, PathBuf::from("未命名 1.md"));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(root.join("未命名.md").is_file());
    assert!(root.join("未命名 1.md").is_file());
    std::fs::remove_dir_all(root).unwrap();
}
#[gpui::test]
fn restores_split_orientation_document_and_independent_modes(cx: &mut TestAppContext) {
    use inkstone_core::preferences::{Preferences, ViewState};
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-split-session-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "# A\nbody").unwrap();
    std::fs::write(root.join("b.md"), "# B\nbody").unwrap();
    Preferences {
        open_paths: vec!["a.md".into(), "b.md".into()],
        active_path: Some("b.md".into()),
        main_path: Some("a.md".into()),
        split_view: Some(ViewState {
            path: "b.md".into(),
            reading: true,
            selection: 2..2,
            folded_lines: vec![0],
            ..Default::default()
        }),
        split_vertical: true,
        split_focused: true,
        views: vec![ViewState {
            path: "a.md".into(),
            live: false,
            selection: 1..1,
            folded_lines: vec![0],
            ..Default::default()
        }],
        ..Default::default()
    }
    .save(&root.join(".inkstone-workspace.json"))
    .unwrap();
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert!(
                w.views.vertical,
                "status={} prefs={:?}",
                w.status, w.ui.prefs
            );
            assert!(w.views.secondary_focused);
            assert_eq!(w.tabs[w.main_tab().unwrap()].path, PathBuf::from("a.md"));
            assert!(!w.tabs[0].pane.read(cx).live);
            assert_eq!(
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .folded_ranges()
                    .len(),
                1
            );
            let pane = w.current_pane().unwrap();
            assert!(pane.read(cx).reading);
            assert_eq!(pane.read(cx).editor.read(cx).selected_range(), 2..2);
            assert_eq!(pane.read(cx).editor.read(cx).folded_ranges().len(), 1);
            assert!(!w.tabs[1].pane.read(cx).reading);
            assert!(
                w.tabs[1]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .folded_ranges()
                    .is_empty()
            );
            w.snapshot_views(cx);
            assert_eq!(w.ui.prefs.views[0].folded_lines, vec![0]);
            assert_eq!(
                w.ui.prefs.split_view.as_ref().unwrap().folded_lines,
                vec![0]
            );
            w.watcher = None;
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn dirty_tab_closes_after_save_and_stays_open_on_conflict(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-close-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(vault.clone());
            w.add_tab("draft.md".into(), None, true, window, cx);
            w.tabs[0]
                .pane
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.replace("不能丢失😀", window, cx));
            w.close_tab(window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert!(w.tabs[0].save.persistence.is_saving());
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("draft.md")).unwrap(),
        "不能丢失😀"
    );
    handle
        .update(cx, |w, window, cx| {
            assert!(w.tabs.is_empty());
            w.add_tab(
                "draft.md".into(),
                Some("不能丢失😀".into()),
                false,
                window,
                cx,
            );
            w.tabs[0]
                .pane
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.replace("本地", window, cx));
            std::fs::write(root.join("draft.md"), "外部").unwrap();
            w.close_tab(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert_eq!(w.tabs.len(), 1);
            assert!(w.tabs[0].save.persistence.has_conflict());
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("draft.md")).unwrap(),
        "外部"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[gpui::test]
fn split_composition_is_not_saved_and_external_changes_require_resolution(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let root = std::env::temp_dir().join(format!("inkstone-split-ime-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), "原文").unwrap();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
            w.split_active(false, window, cx);
            w.current_pane()
                .unwrap()
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| {
                    s.set_selected_range(6..6, cx);
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                "原文"
            );
            assert!(!w.tabs[0].save.persistence.is_dirty());
            w.close_split(window, cx);
            assert!(w.views.split.is_some());
            std::fs::write(root.join("a.md"), "外部更新").unwrap();
            w.refresh(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.tabs[0].save.persistence.has_conflict());
            w.current_pane()
                .unwrap()
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.replace_text_in_range(None, "你", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                "原文你"
            );
            w.save_all(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "外部更新"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn bulk_tab_closing_preserves_pins_anchor_and_conflicts(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            for name in ["a.md", "b.md", "c.md", "d.md"] {
                w.add_tab(name.into(), Some(String::new()), false, window, cx);
            }
            w.tabs[1].pinned = true;
            let a = w.tabs[0].id;
            let c = w.tabs[2].id;
            w.close_tab_group(Some(c), 1, window, cx);
            assert_eq!(w.tabs.len(), 3);
            assert_eq!(w.tabs[w.active.unwrap()].id, c);
            w.close_tab_group(Some(a), 0, window, cx);
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[w.active.unwrap()].id, a);
            w.tabs[0].save.persistence.test_set_conflict(true);
            w.close_tab_group(None, 2, window, cx);
            assert_eq!(w.tabs.len(), 2);
            w.tabs[0].save.persistence.test_set_conflict(false);
            w.close_tab_group(None, 2, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert_eq!(w.tabs[0].path, PathBuf::from("b.md"));
            w.ui.close_pending.insert(w.tabs[0].id);
            w.finish_pending_closes(window, cx);
            assert!(w.ui.close_pending.is_empty());
            w.tabs[0].pinned = false;
            w.finish_pending_closes(window, cx);
            assert_eq!(w.tabs.len(), 1);
        })
        .unwrap();
}

#[gpui::test]
fn automatic_file_reveal_expands_parents_and_can_be_disabled(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            let mut index = Index::default();
            index.update("one/deep/a.md".into(), "A".into());
            index.update("two/b.md".into(), "B".into());
            w.index = Arc::new(index);
            w.sync_index_ui(cx);
            w.add_tab("one/deep/a.md".into(), Some("A".into()), false, window, cx);
            assert!(w.tree.read(cx).index_of(&"one/deep/a.md".into()).is_none());
            w.ui.prefs.auto_reveal_file = true;
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(
                w.tree
                    .read(cx)
                    .selected_item()
                    .unwrap()
                    .id
                    .replace('\\', "/"),
                "one/deep/a.md"
            );
            w.ui.prefs.auto_reveal_file = false;
            w.add_tab("two/b.md".into(), Some("B".into()), false, window, cx);
        })
        .unwrap();
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, window, cx| {
            assert_eq!(
                w.tree
                    .read(cx)
                    .selected_item()
                    .unwrap()
                    .id
                    .replace('\\', "/"),
                "one/deep/a.md"
            );
            w.ui.prefs.left_open = false;
            w.ui.left_mode = 1;
            w.execute_command(74, window, cx);
            assert!(w.ui.prefs.left_open);
            assert_eq!(w.ui.left_mode, 0);
            assert_eq!(
                w.tree
                    .read(cx)
                    .selected_item()
                    .unwrap()
                    .id
                    .replace('\\', "/"),
                "two/b.md"
            );
            assert!(!w.ui.prefs.auto_reveal_file);
        })
        .unwrap();
}

#[gpui::test]
fn many_tabs_keep_header_bounded_and_reveal_active_tab(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            for i in 0..12 {
                w.add_tab(
                    format!("很长的标签名称-{i}.md").into(),
                    Some(String::new()),
                    false,
                    window,
                    cx,
                );
            }
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(600.)));
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, _, _| {
            let bounds = w.ui.tab_scroll.bounds();
            assert!(
                bounds.size.width > px(0.) && bounds.right() < px(800.),
                "{bounds:?}"
            );
            assert!(
                w.ui.tab_scroll.offset().x < px(0.),
                "tabs={}, selected={:?}, bounds={:?}, offset={:?}, last={:?}",
                w.tabs.len(),
                w.main_tab(),
                bounds,
                w.ui.tab_scroll.offset(),
                w.ui.tab_scroll.bounds_for_item(11)
            );
        })
        .unwrap();
    handle
        .update(&mut visual, |w, window, cx| w.focus_primary(0, window, cx))
        .unwrap();
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |w, _, _| {
            assert!(
                w.ui.tab_scroll.offset().x >= px(-5.),
                "{:?}",
                w.ui.tab_scroll.offset()
            );
        })
        .unwrap();
}

#[gpui::test]
fn sidebars_follow_restored_widths_and_keep_them_after_window_resize(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(820.)));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    handle
        .update(&mut visual, |workspace, _, cx| {
            let root = std::env::temp_dir()
                .join(format!("inkstone-sidebar-layout-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            workspace.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
            workspace.ui.prefs.left_open = true;
            workspace.ui.prefs.right_open = true;
            workspace.ui.prefs.left_width = 255.;
            workspace.ui.prefs.right_width = 345.;
            cx.notify();
        })
        .unwrap();
    for width in [1200., 1600., 1200.] {
        visual.simulate_resize(size(px(width), px(820.)));
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert_eq!(
            visual
                .debug_bounds("workspace-left-panel")
                .unwrap()
                .size
                .width,
            px(255.)
        );
        assert_eq!(
            visual
                .debug_bounds("workspace-right-panel")
                .unwrap()
                .size
                .width,
            px(345.)
        );
    }
}

#[gpui::test]
fn completion_cache_reuses_body_edits_and_invalidates_target_changes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, _, _| {
            let from = std::path::Path::new("source.md");
            let mut index = Index::default();
            index.update("target.md".into(), "# Heading\n\nbody ^block\n".into());
            w.index = Arc::new(index);
            let before = w.cached_link_paths(from);
            Arc::make_mut(&mut w.index).update(
                "target.md".into(),
                "extra body\n\n# Heading\n\nchanged ^block\n".into(),
            );
            assert!(Arc::ptr_eq(&before, &w.cached_link_paths(from)));
            let mut previous = before;
            for text in [
                "# Renamed\n\nbody ^block\n",
                "# Renamed\n\nbody ^new-block\n",
                "---\naliases: [别名]\n---\n# Renamed\n\nbody ^new-block\n",
            ] {
                Arc::make_mut(&mut w.index).update("target.md".into(), text.into());
                let next = w.cached_link_paths(from);
                assert!(!Arc::ptr_eq(&previous, &next));
                assert_eq!(*next, *w.link_paths_for(from));
                previous = next;
            }
            Arc::make_mut(&mut w.index).files.push("image.png".into());
            let assets = w.cached_link_paths(from);
            assert!(!Arc::ptr_eq(&previous, &assets));
            assert!(assets.iter().any(|entry| entry.label == "image.png"));
            w.ui.prefs.use_markdown_links = !w.ui.prefs.use_markdown_links;
            let markdown = w.cached_link_paths(from);
            assert!(!Arc::ptr_eq(&assets, &markdown));
            w.index = Arc::new(
                w.index
                    .relocate(std::path::Path::new("target.md"), None, false),
            );
            assert_eq!(*w.cached_link_paths(from), *w.link_paths_for(from));
        })
        .unwrap();
}

#[gpui::test]
fn completion_link_preferences_keep_full_labels_and_resolve(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, _, _| {
            let mut index = Index::default();
            for name in [
                "a/same.md",
                "same.md",
                "a/sub/中文.2026.md",
                "other/unique.md",
            ] {
                index.update(name.into(), String::new());
            }
            w.index = Arc::new(index);
            let from = std::path::Path::new("a/source.md");
            use inkstone_core::locations::LinkFormat;
            for format in [
                LinkFormat::Shortest,
                LinkFormat::Relative,
                LinkFormat::Absolute,
            ] {
                for markdown in [false, true] {
                    w.ui.prefs.link_format = format;
                    w.ui.prefs.use_markdown_links = markdown;
                    let entries = w.link_paths_for(from);
                    for (path, entry) in w.index.notes.keys().zip(entries.iter()) {
                        assert_eq!(
                            entry.label,
                            path.with_extension("").to_string_lossy().replace('\\', "/")
                        );
                        assert_eq!(entry.markdown, markdown);
                        let resolved = if markdown {
                            w.index.resolve_markdown(from, &entry.target).0
                        } else {
                            w.index.resolve(from, &entry.target)
                        };
                        assert_eq!(
                            resolved,
                            inkstone_core::index::Resolution::Found(path.clone()),
                            "{format:?}: {}",
                            entry.target
                        );
                    }
                }
            }
            Arc::make_mut(&mut w.index).files = [
                "media/photo.png",
                "media/record.mp3",
                "media/中文#1.pdf",
                "data/private.bin",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect();
            w.ui.prefs.link_format = LinkFormat::Shortest;
            w.ui.prefs.use_markdown_links = false;
            let entries = w.link_paths_for(from);
            let photo = entries
                .iter()
                .find(|e| e.label == "media/photo.png")
                .unwrap();
            assert_eq!(photo.target, "photo.png");
            assert!(!photo.markdown);
            assert!(entries.iter().any(|e| e.label == "media/record.mp3"));
            assert!(
                entries
                    .iter()
                    .find(|e| e.label == "media/中文#1.pdf")
                    .unwrap()
                    .markdown
            );
            assert!(!entries.iter().any(|e| e.label.ends_with("private.bin")));
            Arc::make_mut(&mut w.index).update(
                "other/unique.md".into(),
                "---\naliases: [项目别名, 项目别名, a|b]\n---\n".into(),
            );
            let entries = w.link_paths_for(from);
            assert_eq!(
                entries
                    .iter()
                    .filter(|e| e.alias.as_deref() == Some("项目别名"))
                    .count(),
                1
            );
            let alias = entries
                .iter()
                .find(|e| e.alias.as_deref() == Some("项目别名"))
                .unwrap();
            assert_eq!(alias.target, "unique");
            let unsafe_alias = entries
                .iter()
                .find(|e| e.alias.as_deref() == Some("a|b"))
                .unwrap();
            assert!(unsafe_alias.markdown);
            assert_eq!(unsafe_alias.target, "unique.md");
        })
        .unwrap();
}

#[gpui::test]
fn insert_footnote_command_preserves_text_and_undo_selection(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.add_tab(
                "footnote.md".into(),
                Some("中文😀".into()),
                false,
                window,
                cx,
            );
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            editor.update(cx, |s, cx| s.set_selected_range(3..10, cx));
            w.execute_command(72, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "中[^1]文😀\n\n[^1]: \n");
            assert_eq!(editor.read(cx).selected_range(), 7..7);
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
            assert_eq!(editor.read(cx).selected_range(), 3..10);
        })
        .unwrap();
}

#[gpui::test]
fn table_commands_insert_align_remove_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.execute_command(61, window, cx);
            assert!(w.tabs.is_empty());
            w.add_tab("table.md".into(), Some(String::new()), false, window, cx);
            let editor = w.current_pane().unwrap().read(cx).editor.clone();
            w.execute_command(60, window, cx);
            assert!(editor.read(cx).value().contains("| --- | --- |"));
            w.execute_command(63, window, cx);
            assert!(editor.read(cx).value().contains("| --- | --- | --- |"));
            w.execute_command(66, window, cx);
            let aligned = editor.read(cx).value();
            assert!(aligned.contains("| --- | :---: | --- |"));
            w.execute_command(64, window, cx);
            assert!(editor.read(cx).value().contains("| --- | --- |"));
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value(), aligned);
        })
        .unwrap();
}

#[gpui::test]
fn preferences_formatting_and_pinned_tabs_work(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.add_tab("中文.md".into(), Some("中文😀".into()), false, window, cx);
            let editor = workspace.tabs[0].pane.read(cx).editor.clone();
            editor.update(cx, |s, cx| s.set_selected_range(0.."中文😀".len(), cx));
            workspace.execute_command(24, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
            workspace.execute_command(24, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
            assert_eq!(editor.read(cx).selected_range(), 0.."中文😀".len());
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
            assert_eq!(editor.read(cx).selected_range(), 2..2 + "中文😀".len());
            workspace.execute_command(46, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "**==中文😀==**");
            workspace.execute_command(46, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
            workspace.execute_command(53, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "# **中文😀**");
            workspace.execute_command(52, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
            workspace.execute_command(48, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "> **中文😀**");
            workspace.execute_command(48, window, cx);
            workspace.execute_command(51, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), "```\n**中文😀**\n```");
            editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
            assert_eq!(editor.read(cx).selected_range(), 2..2 + "中文😀".len());
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            assert!(workspace.tabs[0].save.persistence.is_dirty());
            workspace.tabs[0].save.persistence.test_set_baseline(Some(
                workspace.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .to_string(),
            ));
            workspace.tabs[0].save.persistence.test_set_dirty(false);
            workspace.execute_command(27, window, cx);
            workspace.close_tab(window, cx);
            assert_eq!(workspace.tabs.len(), 1);
            assert!(!workspace.tabs[0].pinned);
            workspace.close_tab(window, cx);
            assert!(workspace.tabs.is_empty());
            assert_eq!(workspace.ui.closed, vec![PathBuf::from("中文.md")]);
        })
        .unwrap();
}

#[gpui::test]
fn bad_note_does_not_block_vault_loading_or_external_refresh(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-partial-index-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("good.md"), "original").unwrap();
    std::fs::write(root.join("z-bad.md"), [0xff]).unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.vault.is_some());
            w.watcher = None;
            w.watch_events = None;
            assert!(
                w.index
                    .errors
                    .contains_key(std::path::Path::new("z-bad.md"))
            );
            assert!(w.tree_files.contains(&PathBuf::from("z-bad.md")));
            assert_eq!(w.tabs[0].path, PathBuf::from("good.md"));
            std::fs::write(root.join("good.md"), "external update").unwrap();
            w.rescan = true;
            w.refresh(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.tabs[0].save.persistence.baseline().as_deref(),
                Some("external update")
            );
            assert_eq!(w.index.errors.len(), 1);
            std::fs::write(root.join("z-bad.md"), "repaired").unwrap();
            w.changed_paths.insert("z-bad.md".into());
            w.refresh(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert!(w.index.errors.is_empty());
            assert_eq!(
                w.index.notes[std::path::Path::new("z-bad.md")].text,
                "repaired"
            );
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_creation_refresh_preserves_notes_and_discovers_imported_files() {
    let root = std::env::temp_dir().join(format!(
        "inkstone-folder-index-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("old.md"), "# Original").unwrap();
    let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
    let index = Arc::new(Index::build(&vault).unwrap());
    // Any unnecessary read of an existing note now fails UTF-8 decoding.
    std::fs::write(root.join("old.md"), [0xff]).unwrap();
    std::fs::write(root.join(".inkstone-workspace.pending"), "settings").unwrap();
    vault
        .create_folder(std::path::Path::new("parent/empty.md"))
        .unwrap();
    let (same, folders) =
        refresh_created_index(&vault, index.clone(), ["parent/empty.md".into()].into()).unwrap();
    assert!(Arc::ptr_eq(&same, &index));
    assert!(folders.contains(&PathBuf::from("parent/empty.md")));

    std::fs::write(root.join("parent/new.md"), "# Imported\n[[old]]").unwrap();
    std::fs::write(root.join("parent/image.png"), []).unwrap();
    let (imported, _) = refresh_created_index(&vault, same, Default::default()).unwrap();
    assert_eq!(
        imported.notes[std::path::Path::new("old.md")].text,
        "# Original"
    );
    assert_eq!(
        imported.notes[std::path::Path::new("parent/new.md")].text,
        "# Imported\n[[old]]"
    );
    assert!(imported.files.contains(&PathBuf::from("parent/image.png")));

    std::fs::write(root.join("old.md"), "# Changed").unwrap();
    std::fs::remove_file(root.join("parent/new.md")).unwrap();
    let (updated, _) = refresh_created_index(&vault, imported, ["old.md".into()].into()).unwrap();
    assert_eq!(
        updated.notes[std::path::Path::new("old.md")].text,
        "# Changed"
    );
    assert!(
        !updated
            .notes
            .contains_key(std::path::Path::new("parent/new.md"))
    );
    assert!(updated.filenames("new").is_empty());
    assert!(updated.backlinks(std::path::Path::new("old.md")).is_empty());
    assert!(matches!(
        updated.resolve(std::path::Path::new("old.md"), "parent/new"),
        Resolution::Missing(_)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn folder_creation_updates_tree_before_tick_and_reuses_index(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!(
        "inkstone-folder-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("old.md"), "# Original").unwrap();
    std::fs::write(root.join(".inkstone-workspace.json"), "{}").unwrap();
    let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
    let index = Arc::new(Index::build(&vault).unwrap());
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(vault.clone());
            w.index = index.clone();
            w.files = index.note_paths();
            w.sync_index_ui(cx);
            w.watch_events = Some(receiver);
            w.ui.name_mode = Some(ui::NameMode::Folder);
            w.name.update(cx, |input, cx| {
                input.set_value("parent/empty.md", window, cx)
            });
            w.submit_name(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.status, "文件夹已创建");
            assert_eq!(w.ui.pending_file_writes, 0);
            assert!(Arc::ptr_eq(&index, &w.index));
            assert!(!w.refreshing);
            let tree = w.tree.read(cx);
            assert!(
                tree.entry(tree.index_of(&SharedString::from("parent")).unwrap())
                    .unwrap()
                    .is_folder()
            );
            assert!(w.ui.folders.contains(&PathBuf::from("parent/empty.md")));
        })
        .unwrap();
    std::fs::write(root.join("old.md"), [0xff]).unwrap();
    for kind in [
        notify::event::CreateKind::Any,
        notify::event::CreateKind::Folder,
    ] {
        sender
            .send(Ok(notify::Event::new(notify::EventKind::Create(kind))
                .add_path(vault.root.join("parent"))
                .add_path(vault.root.join("parent/empty.md"))))
            .unwrap();
        handle
            .update(cx, |w, window, cx| w.tick(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(Arc::ptr_eq(&index, &w.index));
                assert!(!w.refreshing);
                assert!(!w.refresh_requested);
                assert_eq!(w.status, "文件夹已创建");
            })
            .unwrap();
    }
    handle
        .update(cx, |w, window, cx| {
            w.ui.name_mode = Some(ui::NameMode::Folder);
            w.submit_name(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_ne!(w.status, "文件夹已创建");
            assert_eq!(w.ui.folders.len(), 2);
            assert_eq!(w.ui.pending_file_writes, 0);
            w.tick(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| assert!(Arc::ptr_eq(&index, &w.index)))
        .unwrap();

    // Directory rename/remove used to force a full reread. Reconcile from the listing instead.
    std::fs::write(root.join("old.md"), "# Original").unwrap();
    for kind in [
        notify::EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::Any,
        )),
        notify::EventKind::Remove(notify::event::RemoveKind::Folder),
    ] {
        sender
            .send(Ok(
                notify::Event::new(kind).add_path(vault.root.join("parent"))
            ))
            .unwrap();
        handle
            .update(cx, |w, window, cx| w.tick(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(Arc::ptr_eq(&index, &w.index));
                assert!(!w.refreshing);
                assert_eq!(
                    w.index.notes[std::path::Path::new("old.md")].text,
                    "# Original"
                );
            })
            .unwrap();
    }

    // A note edit batched with directory creation must still be indexed.
    std::fs::write(root.join("old.md"), "# Changed").unwrap();
    std::fs::create_dir(root.join("external")).unwrap();
    sender
        .send(Ok(notify::Event::new(notify::EventKind::Create(
            notify::event::CreateKind::Any,
        ))
        .add_path(vault.root.join("external"))))
        .unwrap();
    sender
        .send(Ok(notify::Event::new(notify::EventKind::Modify(
            notify::event::ModifyKind::Any,
        ))
        .add_path(vault.root.join("old.md"))))
        .unwrap();
    handle
        .update(cx, |w, window, cx| w.tick(window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert_eq!(
                w.index.notes[std::path::Path::new("old.md")].text,
                "# Changed"
            );
            assert!(w.ui.folders.contains(&PathBuf::from("external")));
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn session_restores_open_tabs_active_note_and_empty_folders(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = std::env::temp_dir().join(format!("inkstone-session-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("空文件夹")).unwrap();
    std::fs::write(root.join("a.md"), "# A").unwrap();
    std::fs::write(root.join("b.md"), "# B").unwrap();
    let prefs = inkstone_core::preferences::Preferences {
        open_paths: vec!["b.md".into(), "a.md".into()],
        active_path: Some("b.md".into()),
        light: true,
        font_size: 20.,
        left_panel: 1,
        right_panel: 4,
        search_query: "tag:work".into(),
        tags: inkstone_core::tags::Options {
            show_filter: true,
            query: "work".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    prefs.save(&root.join(".inkstone-workspace.json")).unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, cx| {
            assert_eq!(w.tabs.len(), 2);
            assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("b.md"));
            assert_eq!(w.tabs[0].pane.read(cx).font_size, 20.);
            assert!(w.ui.prefs.light);
            assert_eq!(w.ui.left_mode, 1);
            assert_eq!(w.ui.right_mode, 4);
            assert_eq!(w.ui.tags_filter.read(cx).value().as_ref(), "work");
            assert!(w.fulltext);
            assert_eq!(w.search.read(cx).value().as_ref(), "tag:work");
            let tree = w.tree.read(cx);
            let id: SharedString = "空文件夹".into();
            assert!(tree.entry(tree.index_of(&id).unwrap()).unwrap().is_folder());
            w.watcher = None;
            w.ui.left_mode = 2;
            w.ui.right_mode = 3;
            w.persist_workspace(cx);
        })
        .unwrap();
    cx.run_until_parked();
    let saved =
        inkstone_core::preferences::Preferences::load(&root.join(".inkstone-workspace.json"));
    assert_eq!(saved.left_panel, 2);
    assert_eq!(saved.right_panel, 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn opening_existing_document_replaces_current_view_and_focuses_it(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.add_tab("a.md".into(), Some("A".into()), false, window, cx);
            workspace.add_tab("b.md".into(), Some("B".into()), false, window, cx);
            assert!(
                workspace.tabs[1]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            workspace.open_note("a.md".into(), window, cx);
            assert_eq!(workspace.tabs.len(), 2);
            assert_eq!(workspace.active, Some(1));
            assert_eq!(workspace.tabs[1].path, PathBuf::from("a.md"));
            assert!(
                workspace.tabs[1]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .unwrap();
}

#[gpui::test]
fn ten_thousand_backlinks_leave_the_editor_visible(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "中文正文 😀 e\u{301}\n".repeat(400);
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.add_tab("目标.md".into(), Some(source.clone()), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let editor = handle
        .update(cx, |workspace, _, cx| {
            workspace.tabs[0].pane.read(cx).editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(820.)));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-end shift-left");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let before = editor.read_with(&visual, |state, _| (state.selected_range(), state.cursor()));
    handle
        .update(&mut visual, |workspace, _, cx| {
            workspace.backlinks = (0..10000)
                .map(|i| PathBuf::from(format!("{i:05}.md")))
                .collect();
            cx.notify();
        })
        .unwrap();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    editor.read_with(&visual, |state, _| {
        assert!(state.input_bounds().size.height > px(200.));
        assert_eq!(state.value().as_ref(), source);
        assert_eq!((state.selected_range(), state.cursor()), before);
        let (mut caret, _) = state.cursor_layout().unwrap();
        caret.origin += state.scroll_offset();
        assert!(state.input_bounds().intersects(&caret));
    });
    handle
        .update(&mut visual, |workspace, _, _| {
            assert_eq!(workspace.backlinks.len(), 10000)
        })
        .unwrap();
}

#[gpui::test]
fn missing_link_creation_backlinks_and_search(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-links-test-{stamp}"));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("来源.md"), "# 来源\n\n线索 [[目标]]").unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.load_vault(root.clone(), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            workspace.follow_link("来源.md".into(), "目标".into(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(root.join("目标.md").exists());
    handle
        .update(cx, |workspace, window, cx| workspace.refresh(window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.backlinks, vec![PathBuf::from("来源.md")]);
            workspace.fulltext = true;
            workspace.search.update(cx, |state, cx| {
                state.replace_text_in_range(None, "线索", window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    handle
        .update(cx, |workspace, _, _| {
            assert_eq!(workspace.search_results.len(), 1);
            assert_eq!(workspace.search_results[0].path, PathBuf::from("来源.md"));
        })
        .unwrap();
    handle
        .update(cx, |workspace, _, cx| {
            let pane = workspace.tabs[workspace.active.unwrap()].pane.clone();
            pane.update(cx, |_, cx| {
                cx.emit(EditorEvent::FollowMarkdownLink("不会创建.md".into()))
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!root.join("不会创建.md").exists());
    handle
        .update(cx, |workspace, _, cx| {
            assert!(workspace.status.contains("目标不存在"));
            let pane = workspace.tabs[workspace.active.unwrap()].pane.clone();
            pane.update(cx, |_, cx| {
                cx.emit(EditorEvent::FollowMarkdownLink(
                    "%E6%9D%A5%E6%BA%90.md".into(),
                ))
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, _, _| {
            assert_eq!(
                workspace.tabs[workspace.active.unwrap()].path,
                PathBuf::from("来源.md")
            );
            workspace.watcher = None;
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn recovery_panel_refreshes_drafts_created_after_opening_vault(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-recovery-refresh-{stamp}"));
    std::fs::create_dir(&root).unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
        .unwrap();
    cx.run_until_parked();
    let journal = handle
        .update(cx, |w, window, cx| {
            assert!(w.recoveries.is_empty());
            let journal = w
                .vault
                .as_ref()
                .unwrap()
                .journal(std::path::Path::new("草稿.md"), None, "会话中新建的草稿")
                .unwrap();
            w.execute_command(16, window, cx);
            journal
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert!(w.ui.trash_open);
            assert_eq!(w.recoveries.len(), 1);
            assert_eq!(w.recoveries[0].record.draft, "会话中新建的草稿");
            std::fs::rename(&journal, journal.with_extension("saved")).unwrap();
            w.execute_command(16, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, _, _| {
            assert!(w.recoveries.is_empty());
            w.watcher = None;
        })
        .unwrap();
    std::fs::remove_file(journal.with_extension("saved")).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn recovery_opens_exact_draft_as_new_note_without_overwriting_original(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-recovery-ui-{stamp}"));
    std::fs::create_dir(&root).unwrap();
    let original = "# 原件\r\n磁盘版本 😀\r\n";
    let draft = "# 原件\r\n本地草稿 👩‍💻 e\u{301} **保留格式**\r\n";
    std::fs::write(root.join("原件.md"), original).unwrap();
    let vault = Vault::open(root.clone(), app_dir().join("recovery")).unwrap();
    let journal = vault
        .journal(std::path::Path::new("原件.md"), Some(original), draft)
        .unwrap();
    drop(vault);

    // A fresh workspace reads the durable journal, as it would after restart.
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.load_vault(root.clone(), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.recoveries.len(), 1);
            workspace.review_draft(0, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            workspace.finish_draft_review(false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let restored = handle
        .update(cx, |workspace, _, cx| {
            let tab = &workspace.tabs[workspace.active.unwrap()];
            assert_ne!(tab.path, std::path::Path::new("原件.md"));
            assert!(
                !tab.save.persistence.is_dirty()
                    && !tab.save.persistence.is_saving()
                    && tab.save.persistence.error().is_none()
            );
            assert_eq!(tab.pane.read(cx).editor.read(cx).value().as_ref(), draft);
            let path = root.join(&tab.path);
            workspace.watcher = None;
            path
        })
        .unwrap();
    assert_eq!(std::fs::read_to_string(restored).unwrap(), draft);
    assert_eq!(
        std::fs::read_to_string(root.join("原件.md")).unwrap(),
        original
    );
    assert!(
        !journal.exists(),
        "successful recovery retires only the selected draft"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn gpui_create_edit_save_reopen_and_external_conflict(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-ui-vault-{stamp}"));
    std::fs::create_dir(&root).unwrap();
    let handle = cx.add_window(Workspace::new);
    handle
        .update(cx, |workspace, window, cx| {
            workspace.load_vault(root.clone(), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            assert!(workspace.vault.is_some());
            workspace
                .name
                .update(cx, |state, cx| state.set_value("测试.md", window, cx));
            workspace.create_note(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(root.join("测试.md")).unwrap(), "");
    handle
        .update(cx, |workspace, window, cx| {
            let editor = workspace.tabs[0].pane.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.replace_text_in_range(None, "# 中文😀\r\n**原文**\r\n", window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            assert!(workspace.tabs[0].save.persistence.is_dirty());
            workspace.save_all(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let persisted = std::fs::read_to_string(root.join("测试.md")).unwrap();
    assert_eq!(persisted, "# 中文😀\r\n**原文**\r\n");
    handle
        .update(cx, |workspace, window, cx| {
            assert!(!workspace.tabs[0].save.persistence.is_dirty());
            workspace.tabs.clear();
            workspace.active = None;
            workspace.open_note(PathBuf::from("测试.md"), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, window, cx| {
            let editor = workspace.tabs[0].pane.read(cx).editor.clone();
            assert_eq!(editor.read(cx).value().as_ref(), persisted);
            editor.update(cx, |state, cx| {
                state.replace_text_in_range(None, "本地修改", window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::write(root.join("测试.md"), "外部修改").unwrap();
    handle
        .update(cx, |workspace, window, cx| workspace.save_all(window, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |workspace, _, cx| {
            assert!(workspace.tabs[0].save.persistence.has_conflict());
            assert!(workspace.tabs[0].save.persistence.is_dirty());
            assert!(
                workspace.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("本地修改")
            );
            assert!(workspace.status.contains("外部版本"));
            workspace.watcher = None;
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("测试.md")).unwrap(),
        "外部修改"
    );
    std::fs::remove_dir_all(root).unwrap();
}
