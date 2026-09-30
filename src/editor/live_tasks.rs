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
                        .rounded(px(4.))
                        .border_color(rgb(if checked {
                            0x8b6cef
                        } else if light {
                            0xababab
                        } else {
                            0x666666
                        }))
                        .when(checked, |view| {
                            view.bg(rgb(0x8b6cef)).child(
                                svg()
                                    .path("icons/check.svg")
                                    .size(px(font_size - 2.))
                                    .text_color(rgb(if light { 0xffffff } else { 0x1e1e1e })),
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
    use core::prelude::v1::test;

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
        visual.simulate_keystrokes("ctrl-z");
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
        visual.simulate_keystrokes("ctrl-z");
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
