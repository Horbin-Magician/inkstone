use super::*;

pub(super) fn overlay(
    editor: Entity<EditorState>,
    markers: Vec<std::ops::Range<usize>>,
    font_size: f32,
    light: bool,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let baseline = editor.read(cx).value();
            let visible: Vec<_> = markers
                .iter()
                .filter_map(|range| {
                    let bounds = editor.read(cx).range_to_bounds(range)?;
                    (bounds.bottom() > viewport.top()
                        && bounds.top() < viewport.bottom()
                        && bounds.right() > viewport.left())
                    .then_some((range.clone(), bounds))
                })
                .collect();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for (range, bounds) in visible {
                    let offset = range.start;
                    let editor = editor.clone();
                    let baseline = baseline.clone();
                    let width = bounds.size.width;
                    let mut element = div()
                        .id(("live-list", offset))
                        .debug_selector(move || format!("live-list-{offset}"))
                        .w(width)
                        .h(bounds.size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .block_mouse_except_scroll()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            editor.update(cx, |state, cx| {
                                if state.value() != baseline {
                                    return;
                                }
                                state.set_selected_range(offset..offset + 1, cx);
                                state.focus(window, cx);
                            });
                        })
                        .child(
                            div()
                                .size(px(font_size * 0.25))
                                .rounded_full()
                                .bg(crate::theme::palette(light).foreground),
                        )
                        .into_any_element();
                    element.prepaint_as_root(
                        bounds.origin,
                        size(width, bounds.size.height).into(),
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
    fn bullets_render_reveal_and_leave_tasks_and_source_intact(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source =
            "* 中文\n  - nested\n\n+ other\n\n> * quoted\n\n* [ ] task\n\n```\n* code\n```\n\nend";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.update_presentation(cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.live_lists.len(), 4);
                assert_eq!(p.live_tasks.len(), 1);
                let concealed = p.editor.read(cx).concealed_ranges();
                for marker in &p.live_lists {
                    assert!(
                        concealed.contains(marker),
                        "the source glyph must be omitted, not just styled transparent"
                    );
                }
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
        let bullet = visual.debug_bounds("live-list-0").unwrap();
        assert!(
            (f32::from(bullet.size.width) - 16. * 0.6).abs() <= 0.5,
            "bullet bounds: {bullet:?}"
        );
        let nested = visual.debug_bounds("live-list-11").unwrap();
        assert!(nested.left() > bullet.left());
        visual.simulate_click(bullet.center(), Modifiers::default());
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-list-0").is_none());
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).selected_range(), 0..1);
                assert!(!p.editor.read(cx).concealed_ranges().contains(&(0..1)));
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.set_search_query("*", false, cx);
                });
                p.update_presentation(cx);
                assert_eq!(p.live_lists.len(), 2);
                assert!(!p.editor.read(cx).concealed_ranges().contains(&(0..1)));
                p.editor.update(cx, |s, cx| s.close_search(cx));
                p.update_presentation(cx);
                assert_eq!(p.live_lists.len(), 4);
                p.live = false;
                p.update_presentation(cx);
                assert!(p.live_lists.is_empty());
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
}
