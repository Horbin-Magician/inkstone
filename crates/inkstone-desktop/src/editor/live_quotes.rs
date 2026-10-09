use super::*;

pub(super) fn inset(rem_size: Pixels) -> Pixels {
    // Match the reading view: border_l_3 followed by px_4 (one rem).
    px(3.) + rem_size
}

/// Reading-mode list items indent continuation blocks by one rem, independent
/// of how many source spaces the author used.
pub(super) fn list_indent(rem_size: Pixels) -> Pixels {
    rem_size
}

pub(super) fn overlay(
    editor: Entity<EditorState>,
    lines: Vec<std::ops::Range<usize>>,
    indents: Vec<Pixels>,
    text: SharedString,
    rem_size: Pixels,
    light: bool,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let inset = inset(rem_size);
            let borders: Vec<_> = lines
                .iter()
                .zip(indents)
                .filter_map(|(range, indent)| {
                    let state = editor.read(cx);
                    let bounds = state.range_to_bounds(range)?;
                    if !viewport.intersects(&bounds) {
                        return None;
                    }
                    // Source spaces before a list quote already move the range.
                    // Reading mode indents by list depth instead, so cancel only
                    // that leading space. Nested `>` markers stay in place.
                    let line = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
                    let spaces = text[line..range.start]
                        .bytes()
                        .take_while(|b| matches!(b, b' ' | b'\t'))
                        .count();
                    let shift = if spaces == 0 {
                        px(0.)
                    } else {
                        state
                            .range_to_bounds(&(line + spaces..line + spaces))
                            .zip(state.range_to_bounds(&(line..line)))
                            .map(|(content, line)| content.left() - line.left())
                            .unwrap_or(px(0.))
                    };
                    Some((range.start, bounds, indent - shift))
                })
                .collect();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for (offset, bounds, indent) in borders {
                    let mut border = div()
                        .id(("live-quote", offset))
                        .debug_selector(move || format!("live-quote-{offset}"))
                        .w(px(3.))
                        .h(bounds.size.height)
                        .bg(crate::theme::palette(light).border)
                        .into_any_element();
                    border.prepaint_as_root(
                        bounds.origin - point(inset, px(0.)) + point(indent, px(0.)),
                        size(px(3.), bounds.size.height).into(),
                        window,
                        cx,
                    );
                    elements.push(border);
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
                    assert!(pane.editor.read(cx).concealed_ranges().contains(&(0..2)));
                    assert_eq!(pane.editor.read(cx).value().as_ref(), source);
                })
                .unwrap();
        }
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let border = visual.debug_bounds("live-quote-0").unwrap();
        handle
            .update(&mut visual, |pane, _, cx| {
                let viewport = pane.editor.read(cx).input_bounds();
                assert!(border.left() >= viewport.left());
                let text = pane.editor.read(cx).range_to_bounds(&(2..2)).unwrap();
                assert_eq!(text.left() - border.left(), inset(pane.quote_rem_size));
                pane.editor
                    .update(cx, |state, cx| state.set_selected_range(0..0, cx));
                pane.update_presentation(cx);
                assert!(!pane.editor.read(cx).concealed_ranges().contains(&(0..2)));
                pane.live = false;
                pane.update_presentation(cx);
                assert!(pane.live_quotes.is_empty());
                assert!(!pane.editor.read(cx).concealed_ranges().contains(&(0..2)));
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
                let actual = p.editor.read(cx).range_to_bounds(&(8..8)).unwrap();
                assert_eq!(actual.origin.x, position.origin.x - inset(p.quote_rem_size));
                assert_eq!(actual.origin.y, position.origin.y);
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
        assert_eq!(first.size.width, px(3.));
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
                let state = p.editor.read(cx);
                let prefix_width = state.range_to_bounds(&(2..2)).unwrap().left()
                    - state.range_to_bounds(&(0..0)).unwrap().left();
                assert_eq!(
                    state.range_to_bounds(&(2..2)).unwrap().left(),
                    text_x - inset(p.quote_rem_size) + prefix_width
                );
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn list_quote_borders_use_reading_indent(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "1. > 首行引用\n\n- > 首行项目\n\n1. 标题\n   > 后续引用\n   续行\n\n- 项目\n  - 嵌套\n    > 深层\n    深层续行\n\n> 顶层\n续行\n";
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
        visual.simulate_resize(size(px(900.), px(700.)));
        for _ in 0..6 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let (quotes, indents) = handle
            .update(&mut visual, |p, _, _| {
                (p.live_quotes.clone(), p.live_quote_indents.clone())
            })
            .unwrap();
        let borders: Vec<_> = quotes
            .iter()
            .map(|range| {
                let id = Box::leak(format!("live-quote-{}", range.start).into_boxed_str());
                visual.debug_bounds(id).unwrap()
            })
            .collect();
        let step = handle
            .update(&mut visual, |p, _, _| list_indent(p.quote_rem_size))
            .unwrap();
        // Same levels measured against the reading blockquote: one rem per list.
        let expected = [step, step, step, step, step * 2., step * 2., px(0.), px(0.)];
        assert_eq!(indents, expected);
        // A quote opening a list item keeps that item marker's column. A later
        // quote in the item is indented one rem per list, as in reading mode.
        let column = borders[6].left();
        // Opening quotes keep their marker column: "1. " is wider than "- ".
        assert!(borders[0].left() > borders[1].left());
        assert!(borders[1].left() > column + step);
        assert_eq!(borders[2].left(), borders[3].left());
        assert_eq!(borders[2].left(), column + step);
        assert_eq!(borders[4].left(), borders[5].left());
        assert_eq!(borders[4].left(), column + step * 2.);
        // A lazy continuation shares its marker line's rail, not column zero.
        assert_eq!(borders[2].left(), borders[3].left());
        assert_eq!(borders[2].bottom(), borders[3].top());
        assert_eq!(borders[4].left(), borders[5].left());
        assert_eq!(borders[6].left(), borders[7].left());
        handle
            .update(&mut visual, |p, _, cx| {
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
                assert!(border.left() >= viewport.left());
                assert!(border.right() <= viewport.right());
                for (offset, _) in source[..source.find('\n').unwrap()].char_indices() {
                    let glyph = p
                        .editor
                        .read(cx)
                        .range_to_bounds(&(offset..offset))
                        .unwrap();
                    assert!(glyph.left() - border.right() >= p.quote_rem_size);
                }
                assert!(border.bottom() > viewport.top());
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
}
