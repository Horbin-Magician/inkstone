use core::prelude::v1::test;
use gpui::{prelude::*, *};
use gpui_base::input::{DisplayObject, EditorState};
use gpui_component::input::Editor;

struct Root(Entity<EditorState>);
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Editor::new(&self.0).size_full())
    }
}

#[gpui::test]
fn object_bounds_anchor_at_start_when_source_wraps(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = format!("prefix {} tail", "中文😀 ".repeat(25));
    let handle = cx.add_window(|window, cx| {
        Root(cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(source.clone())
                .line_number(false)
                .soft_wrap(true)
        }))
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(240.), px(500.)));
    handle
        .update(&mut visual, |root, _, cx| {
            root.0.update(cx, |state, cx| {
                state.set_display_objects(
                    &source,
                    vec![DisplayObject {
                        id: 3,
                        source: 7..source.len() - 5,
                        size: size(px(100.), px(40.)),
                        baseline: None,
                    }],
                    cx,
                );
            });
        })
        .unwrap();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    handle
        .update(&mut visual, |root, _, cx| {
            let state = root.0.read(cx);
            let start = state.range_to_bounds(&(7..7)).unwrap();
            let source_bounds = state.range_to_bounds(&(7..source.len() - 5)).unwrap();
            assert!(source_bounds.size.height > start.size.height);
            assert_eq!(state.display_object_bounds(3).unwrap().origin, start.origin);
            assert_eq!(state.value().as_ref(), source);
        })
        .unwrap();
}

#[gpui::test]
fn object_height_changes_restore_the_visible_source_anchor(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = format!("before\n$$\nx^2\n$$\nafter\n{}", "tail\n".repeat(100));
    let handle = cx.add_window(|w, cx| {
        Root(cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source.clone())
                .line_number(false)
        }))
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let install = |state: &mut EditorState, height, cx: &mut Context<EditorState>| {
        let start = source.find("$$").unwrap();
        let end = source.rfind("$$").unwrap() + 2;
        state.set_display_objects(
            &source,
            vec![DisplayObject {
                id: 1,
                source: start..end,
                size: size(px(100.), px(height)),
                baseline: None,
            }],
            cx,
        );
        let projection = state.display_projection(state.line_height().unwrap_or(px(24.)), vec![]);
        state.set_line_typography(projection.typography, cx);
        state.set_concealed_lines(projection.hidden_lines, cx);
        state.set_concealed_ranges_with_widths(projection.replacements, cx);
    };
    handle
        .update(&mut visual, |root, _, cx| {
            root.0.update(cx, |s, cx| {
                install(s, 240., cx);
                s.set_scroll_offset(point(px(0.), px(-360.)), cx);
            })
        })
        .unwrap();
    for _ in 0..3 {
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let anchor = handle
        .update(&mut visual, |root, _, cx| {
            root.0.read(cx).display_scroll_anchor().unwrap()
        })
        .unwrap();
    handle
        .update(&mut visual, |root, _, cx| {
            root.0.update(cx, |s, cx| install(s, 480., cx))
        })
        .unwrap();
    for _ in 0..3 {
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |root, _, cx| {
            root.0.update(cx, |s, cx| {
                assert!(s.restore_display_scroll_anchor(anchor, cx));
            })
        })
        .unwrap();
    for _ in 0..3 {
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |root, _, cx| {
            let restored = root.0.read(cx).display_scroll_anchor().unwrap();
            assert_eq!(restored.source_offset(), anchor.source_offset());
            assert!(
                (f32::from(restored.viewport_y() - anchor.viewport_y())).abs() < 1.,
                "before={:?}, after={:?}, scroll={:?}",
                anchor.viewport_y(),
                restored.viewport_y(),
                root.0.read(cx).scroll_offset()
            );
            assert_eq!(root.0.read(cx).value().as_ref(), source);
        })
        .unwrap();
}

#[gpui::test]
fn display_objects_project_crlf_and_variable_heights_without_changing_content_or_history(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let source = "before\r\n$$\r\nx^2\r\n$$\r\nafter😀\r\n";
    let window = cx.add_window(|w, cx| {
        Root(cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source)
                .line_number(false)
        }))
    });
    window
        .update(cx, |root, _, cx| {
            root.0.update(cx, |s, cx| {
                let start = source.find("$$").unwrap();
                let end = source.rfind("$$").unwrap() + 2;
                assert!(s.set_display_objects(
                    source,
                    vec![DisplayObject {
                        id: 1,
                        source: start..end,
                        size: size(px(100.), px(240.)),
                        baseline: None
                    }],
                    cx
                ));
                let p = s.display_projection(px(24.), vec![]);
                assert_eq!(p.replacements[0].0, start..start + 2);
                assert_eq!(
                    p.hidden_lines,
                    vec![source.find("x^2").unwrap(), source.rfind("$$").unwrap()]
                );
                s.set_line_typography(p.typography, cx);
                s.set_concealed_lines(p.hidden_lines, cx);
                s.set_concealed_ranges_with_widths(p.replacements, cx);
                assert_eq!(s.value().as_ref(), source);
            })
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    for _ in 0..3 {
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    window
        .update(&mut visual, |root, w, cx| {
            root.0.update(cx, |s, cx| {
                assert!(s.display_object_bounds(1).unwrap().size.height >= px(240.));
                s.set_selected_range(0..source.len(), cx);
                assert_eq!(s.selected_text().to_string(), source);
                s.set_selected_range(0..0, cx);
                s.replace("changed", w, cx);
                assert!(s.display_objects().is_empty());
                s.undo(&gpui_base::input::Undo, w, cx);
                assert_eq!(s.value().as_ref(), source);
                assert!(!s.set_display_objects("stale", vec![], cx));
            })
        })
        .unwrap();
}
