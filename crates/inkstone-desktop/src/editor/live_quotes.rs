use super::*;

pub(super) fn overlay(
    editor: Entity<EditorState>,
    lines: Vec<std::ops::Range<usize>>,
    font_size: f32,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            // Keep the rail in the pane's left inset so soft-wrapped and lazy
            // continuation lines have the same minimum gap as the first row.
            let inset = px((font_size * 0.75).clamp(8., 24.) + 2.);
            let mut clip = viewport;
            clip.origin.x -= inset;
            clip.size.width += inset;
            let borders: Vec<_> = lines
                .iter()
                .filter_map(|range| {
                    let bounds = editor.read(cx).range_to_bounds(range)?;
                    viewport
                        .intersects(&bounds)
                        .then_some((range.start, bounds))
                })
                .collect();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                for (offset, bounds) in borders {
                    let mut border = div()
                        .id(("live-quote", offset))
                        .debug_selector(move || format!("live-quote-{offset}"))
                        .w(px(2.))
                        .h(bounds.size.height)
                        .bg(gpui_component::Theme::global(cx).primary)
                        .into_any_element();
                    border.prepaint_as_root(
                        bounds.origin - point(inset, px(0.)),
                        size(px(2.), bounds.size.height).into(),
                        window,
                        cx,
                    );
                    elements.push(border);
                }
            });
            (elements, clip)
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
    fn quote_prefix_is_hidden_while_editing_body_and_revealed_at_marker(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> 子列：没有数列\n\n正文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let body = source.find('数').unwrap();
        for caret in [source.len(), body] {
            handle
                .update(&mut visual, |pane, _, cx| {
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(caret..caret, cx);
                    });
                    pane.update_presentation(cx);
                    assert!(pane.editor.read(cx).concealed_ranges().contains(&(0..1)));
                    assert_eq!(pane.editor.read(cx).value().as_ref(), source);
                })
                .unwrap();
        }
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let border = visual.debug_bounds("live-quote-0").unwrap();
        handle
            .update(&mut visual, |pane, _, cx| {
                let text = pane.editor.read(cx).range_to_bounds(&(2..2)).unwrap();
                assert!(text.left() - border.right() >= px(pane.font_size * 0.75));
                pane.editor
                    .update(cx, |state, cx| state.set_selected_range(0..0, cx));
                pane.update_presentation(cx);
                assert!(!pane.editor.read(cx).concealed_ranges().contains(&(0..1)));
                pane.live = false;
                pane.update_presentation(cx);
                assert!(pane.live_quotes.is_empty());
                assert!(!pane.editor.read(cx).concealed_ranges().contains(&(0..1)));
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn lazy_quote_border_continues_without_changing_source_positions(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> first\ncontinuation\n\noutside";
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
        let first = visual.debug_bounds("live-quote-0").unwrap();
        let next = visual.debug_bounds("live-quote-8").unwrap();
        assert_eq!(first.bottom(), next.top());
        assert_eq!(first.left(), next.left());
        assert!(visual.debug_bounds("live-quote-22").is_none());
        let position = handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.live_quotes.len(), 2);
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
                let position = p.editor.read(cx).range_to_bounds(&(8..8)).unwrap();
                p.live = false;
                p.update_presentation(cx);
                position
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-quote-8").is_none());
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(
                    p.editor.read(cx).range_to_bounds(&(8..8)).unwrap(),
                    position
                );
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn quote_borders_follow_nested_source_positions_and_source_mode(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> outer\n> > inner\n\nplain";
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
        let first = visual.debug_bounds("live-quote-0").unwrap();
        let outer = visual.debug_bounds("live-quote-8").unwrap();
        let nested = visual.debug_bounds("live-quote-10").unwrap();
        assert_eq!(first.size.width, px(2.));
        assert_eq!(outer.origin.y, nested.origin.y);
        assert!(nested.left() > outer.right());
        let text_x = handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.live_quotes.len(), 3);
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
                let x = p.editor.read(cx).range_to_bounds(&(2..2)).unwrap().left();
                assert!(x > first.right());
                p.live = false;
                p.update_presentation(cx);
                x
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-quote-0").is_none());
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(
                    p.editor.read(cx).range_to_bounds(&(2..2)).unwrap().left(),
                    text_x
                );
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn quote_border_spans_soft_wrapped_lines(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!("> {}\n\nend", "引用正文 ".repeat(30));
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        handle
            .update(cx, |p, _, cx| p.update_presentation(cx))
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(240.), px(600.)));
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let border = visual.debug_bounds("live-quote-0").unwrap();
        handle
            .update(&mut visual, |p, _, cx| {
                assert!(border.size.height > p.editor.read(cx).line_height().unwrap() * 2.);
                let viewport = p.editor.read(cx).input_bounds();
                for (offset, _) in source[..source.find('\n').unwrap()].char_indices() {
                    let glyph = p
                        .editor
                        .read(cx)
                        .range_to_bounds(&(offset..offset))
                        .unwrap();
                    assert!(glyph.left() - border.right() >= px(p.font_size * 0.75 - 1.));
                }
                assert!(border.bottom() > viewport.top());
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
}
