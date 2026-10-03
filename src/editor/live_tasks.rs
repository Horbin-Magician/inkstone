use super::*;

#[derive(Clone)]
pub(super) struct TaskWidget {
    pub range: std::ops::Range<usize>,
    pub target: inkstone::rendering::TaskTarget,
    pub checked: bool,
    pub inset: f32,
}

pub(super) fn overlay(
    pane: WeakEntity<EditorPane>,
    editor: Entity<EditorState>,
    tasks: Vec<TaskWidget>,
    font_size: f32,
    light: bool,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let visible: Vec<_> = tasks
                .iter()
                .filter_map(|task| {
                    let bounds = editor.read(cx).range_to_bounds(&task.range)?;
                    viewport
                        .intersects(&bounds)
                        .then_some((task.clone(), bounds))
                })
                .collect();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for (task, bounds) in visible {
                    let target = task.target.clone();
                    let checked = task.checked;
                    let weak = pane.clone();
                    let id = task.target.marker.start;
                    let label = task
                        .target
                        .baseline
                        .get(task.range.end..)
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_owned();
                    let indicator = div()
                        .size(px(font_size))
                        .border_1()
                        .rounded_full()
                        .border_color(if checked {
                            crate::theme::palette(light).accent
                        } else {
                            crate::theme::palette(light).muted
                        })
                        .when(checked, |view| {
                            view.bg(crate::theme::palette(light).accent).child(
                                svg()
                                    .path("icons/check.svg")
                                    .size(px(font_size - 2.))
                                    .text_color(crate::theme::palette(light).background),
                            )
                        });
                    let mut element = gpui_base::Checkbox::new(("live-task", id))
                        .checked(checked)
                        .accessibility_label(label)
                        .tab_stop(false)
                        .debug_selector(move || format!("live-task-{id}"))
                        .size(px(font_size))
                        .block_mouse_except_scroll()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_change(move |_, _, window, cx| {
                            cx.stop_propagation();
                            window.prevent_default();
                            let _ = weak.update(cx, |pane, cx| {
                                let mut target = target.clone();
                                target.path = pane.current_path.clone();
                                pane.toggle_task(target, !checked, window, cx)
                            });
                        })
                        .child(indicator)
                        .into_any_element();
                    let origin = point(
                        bounds.left() + px(task.inset),
                        bounds.top() + (bounds.size.height - px(font_size)) / 2.,
                    );
                    element.prepaint_as_root(
                        origin,
                        size(px(font_size), px(font_size)).into(),
                        window,
                        cx,
                    );
                    elements.push(element);
                }
            });
            (elements, viewport)
        },
        |_, (mut elements, viewport), window, cx| {
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for element in &mut elements {
                    element.paint(window, cx);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PlatformKeys;
    use core::prelude::v1::test;

    #[gpui::test]
    fn overlay_geometry_is_current_during_prepaint(cx: &mut TestAppContext) {
        struct Harness(Entity<EditorState>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let before = self.0.clone();
                let after = self.0.clone();
                div()
                    .relative()
                    .size_full()
                    .child(Editor::new(&self.0).h_full())
                    .child(
                        canvas(
                            move |_, _, cx| {
                                let state = before.read(cx);
                                (state.input_bounds(), state.range_to_bounds(&(35..35)))
                            },
                            move |_, geometry, _, cx| {
                                let state = after.read(cx);
                                assert_eq!(
                                    geometry,
                                    (state.input_bounds(), state.range_to_bounds(&(35..35))),
                                    "overlay prepaint must see the geometry used to paint text"
                                );
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| {
            Harness(cx.new(|cx| EditorState::new(w, cx).default_value("prefix\n".repeat(100))))
        });
        let editor = handle.update(cx, |h, _, _| h.0.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for offset in [0., -13.5, -30.75, -21., 0., -600., 0.] {
            visual.update(|w, cx| {
                editor.update(cx, |s, cx| {
                    s.set_scroll_offset(point(px(0.), px(offset)), cx)
                });
                w.draw(cx).clear(cx);
            });
        }
    }

    #[gpui::test]
    fn live_overlays_follow_text_when_scrolling(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!(
            "{}* [ ] task\n\n> quote\n\n---\n\n| a | b |\n| --- | --- |\n| c | d |\n\n{}",
            "prefix\n".repeat(5),
            "tail\n".repeat(100),
        );
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..8 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let editor = handle
            .update(&mut visual, |p, _, _| p.editor.clone())
            .unwrap();
        // Both wheel directions, fractional deltas, and a jump that changes
        // the visible line set; the phase-level test above also guards the first frame.
        for delta in [-13.5, -17.25, 9.75, 21., -600., 600.] {
            let before = editor.read_with(&visual, |s, _| s.scroll_offset().y);
            let position = editor.read_with(&visual, |s, _| s.input_bounds().center());
            visual.update(|w, cx| {
                w.dispatch_event(
                    gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                        modifiers: Default::default(),
                        touch_phase: TouchPhase::Moved,
                    }),
                    cx,
                );
                w.draw(cx).clear(cx);
            });
            let after = editor.read_with(&visual, |s, _| s.scroll_offset().y);
            assert_ne!(before, after, "wheel must move the editor");
            let expected = handle
                .update(&mut visual, |p, _, cx| {
                    let state = p.editor.read(cx);
                    let viewport = state.input_bounds();
                    let mut expected = Vec::new();
                    assert_eq!(p.live_tasks.len(), 1);
                    assert_eq!(p.live_rules.len(), 1);
                    assert!(!p.live_quotes.is_empty());
                    assert_eq!(p.live_objects.len(), 1);
                    for task in &p.live_tasks {
                        let bounds = state
                            .range_to_bounds(&task.range)
                            .filter(|b| viewport.intersects(b))
                            .map(|b| {
                                point(
                                    b.left() + px(task.inset),
                                    b.center().y - px(p.font_size / 2.),
                                )
                            });
                        expected.push((format!("live-task-{}", task.target.marker.start), bounds));
                    }
                    for (kind, ranges) in [("rule", &p.live_rules), ("quote", &p.live_quotes)] {
                        for range in ranges {
                            let origin = state
                                .range_to_bounds(range)
                                .filter(|b| viewport.intersects(b))
                                .map(|b| b.origin);
                            expected.push((format!("live-{kind}-{}", range.start), origin));
                        }
                    }
                    for widget in &p.live_objects {
                        let origin = state
                            .display_object_bounds(widget.source.start as u64)
                            .filter(|b| viewport.intersects(b))
                            .map(|b| b.origin);
                        expected.push((format!("live-object-{}", widget.source.start), origin));
                    }
                    expected
                })
                .unwrap();
            for (selector, origin) in expected {
                let selector = Box::leak(selector.into_boxed_str());
                let actual = visual.debug_bounds(selector).map(|b| b.origin);
                match (actual, origin) {
                    (Some(actual), Some(expected)) => assert!(
                        (actual.x - expected.x).abs() <= px(0.5)
                            && (actual.y - expected.y).abs() <= px(0.5),
                        "{selector} after wheel delta {delta}: {actual:?} != {expected:?}"
                    ),
                    _ => assert_eq!(actual, origin, "{selector} after wheel delta {delta}"),
                }
            }
        }
    }

    #[gpui::test]
    fn folded_and_scrolled_tasks_do_not_leave_controls_on_other_lines(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!(
            "# Section\n- [ ] hidden\n# Next\n- [ ] visible\n{}",
            "tail\n".repeat(80)
        );
        let first = source.find("[ ]").unwrap() + 1;
        let last = source.rfind("[ ]").unwrap() + 1;
        let first_selector = Box::leak(format!("live-task-{first}").into_boxed_str());
        let last_selector = Box::leak(format!("live-task-{last}").into_boxed_str());
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        handle
            .update(cx, |p, _, cx| p.update_presentation(cx))
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let old_position = visual.debug_bounds(first_selector).unwrap().center();
        assert!(visual.debug_bounds(last_selector).is_some());
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| s.restore_fold_lines(&[0], cx));
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds(first_selector).is_none());
        assert!(visual.debug_bounds(last_selector).is_some());
        visual.simulate_click(old_position, Modifiers::default());
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
                p.editor.update(cx, |s, cx| s.restore_fold_lines(&[], cx));
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds(first_selector).is_some());
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_scroll_offset(point(px(0.), px(-400.)), cx)
                });
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds(first_selector).is_none());
        assert!(visual.debug_bounds(last_selector).is_none());
        handle
            .update(&mut visual, |p, _, cx| {
                assert!(
                    p.editor
                        .read(cx)
                        .range_to_bounds(&(first - 1..first + 2))
                        .is_none()
                );
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn live_checkbox_preserves_multiple_selections_and_allows_scrolling(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!("- [✓] item\nword word\n{}", "正文\n".repeat(100));
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let editor = handle
            .update(cx, |p, window, cx| {
                p.editor.update(cx, |s, cx| {
                    let first = source.find("word").unwrap();
                    s.set_selected_range(first..first + 4, cx);
                    s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, window, cx);
                    assert!(s.has_multiple_selections());
                    s.focus(window, cx);
                });
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let selections = editor.read_with(&visual, |s, _| s.selected_ranges());
        let position = visual.debug_bounds("live-task-3").unwrap().center();
        visual.simulate_click(position, Modifiers::default());
        visual.run_until_parked();
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), source.replacen("[✓]", "[ ]", 1));
            assert_eq!(
                s.selected_ranges(),
                selections
                    .iter()
                    .map(|r| r.start - 2..r.end - 2)
                    .collect::<Vec<_>>()
            );
        });
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let position = visual.debug_bounds("live-task-3").unwrap().center();
        visual.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        visual.update(|w, cx| w.draw(cx).clear(cx));
        editor.read_with(&visual, |s, _| assert!(s.scroll_offset().y < px(0.)));
        visual.simulate_platform_keystrokes("ctrl-z");
        visual.run_until_parked();
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), source);
            assert_eq!(s.selected_ranges(), selections);
        });
    }

    #[gpui::test]
    fn live_checkbox_click_is_undoable_and_source_modes_reveal_markers(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "- [ ] task 中文\n- [x] done\n1. [ ] ordered\n```\n- [ ] code\n```\nend";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, window, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.focus(window, cx);
                });
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, _, _| {
                assert_eq!(p.live_tasks.len(), 3);
                assert_eq!(
                    p.live_tasks[2].range.start,
                    source.find("[ ] ordered").unwrap()
                );
            })
            .unwrap();
        let checkbox = visual.debug_bounds("live-task-3").unwrap();
        assert_eq!(checkbox.size, size(px(16.), px(16.)));
        visual.simulate_click(checkbox.center(), Modifiers::default());
        visual.run_until_parked();
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.value().as_ref(), source.replacen("[ ]", "[x]", 1));
            assert_eq!(state.selected_range(), source.len()..source.len());
        });
        visual.simulate_platform_keystrokes("ctrl-z");
        visual.run_until_parked();
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.value().as_ref(), source)
        });
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
                p.update_presentation(cx);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-task-3").is_none());
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.set_search_query("[ ]", false, cx);
                });
                p.update_presentation(cx);
                assert!(p.live_tasks.iter().all(|task| task.checked));
                p.editor.update(cx, |s, cx| s.close_search(cx));
                p.font_size = 20.;
                p.update_presentation(cx);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert_eq!(
            visual.debug_bounds("live-task-3").unwrap().size,
            size(px(20.), px(20.))
        );
        handle
            .update(&mut visual, |p, _, cx| {
                p.live = false;
                p.update_presentation(cx);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-task-3").is_none());
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.value().as_ref(), source)
        });
    }
}
