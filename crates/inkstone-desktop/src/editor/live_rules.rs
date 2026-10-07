use super::*;

pub(super) fn overlay(
    editor: Entity<EditorState>,
    rules: Vec<std::ops::Range<usize>>,
    light: bool,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let text_bounds = editor.read(cx).text_bounds().unwrap_or(viewport);
            let revision = editor.read(cx).text_revision();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for range in &rules {
                    let Some(bounds) = editor.read(cx).range_to_bounds(range) else {
                        continue;
                    };
                    if !(bounds.bottom() > viewport.top()
                        && bounds.top() < viewport.bottom()
                        && text_bounds.right() > bounds.left())
                    {
                        continue;
                    }
                    let offset = range.start;
                    let editor = editor.clone();
                    let width = text_bounds.right() - bounds.left();
                    let mut element = div()
                        .id(("live-rule", offset))
                        .debug_selector(move || format!("live-rule-{offset}"))
                        .w(width)
                        .h(bounds.size.height)
                        .flex()
                        .items_center()
                        .block_mouse_except_scroll()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            editor.update(cx, |state, cx| {
                                if state.text_revision() != revision {
                                    return;
                                }
                                let baseline = state.value();
                                let start = baseline[..offset].rfind('\n').map_or(0, |i| i + 1);
                                let end = baseline[offset..]
                                    .find(['\r', '\n'])
                                    .map_or(baseline.len(), |i| offset + i);
                                state.set_selected_range(start..end, cx);
                                state.focus(window, cx);
                            });
                        })
                        .child(div().w_full().h(px(2.)).bg(rgb(if light {
                            0xe6e6e6
                        } else {
                            0x363636
                        })))
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
    fn rule_click_reveals_source_and_search_keeps_it_editable(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "text\n\n---\n\nend";
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
        let rule = visual.debug_bounds("live-rule-6").unwrap();
        assert!(rule.size.width > px(100.));
        visual.simulate_click(rule.center(), Modifiers::default());
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("live-rule-6").is_none());
        handle
            .update(&mut visual, |p, window, cx| {
                assert_eq!(p.editor.read(cx).selected_range(), 6..9);
                p.editor.update(cx, |s, cx| {
                    s.undo(&gpui_component::input::Undo, window, cx);
                    assert_eq!(s.value().as_ref(), source);
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.set_search_query("---", false, cx);
                });
                p.update_presentation(cx);
                assert!(p.live_rules.is_empty());
                p.editor.update(cx, |s, cx| s.close_search(cx));
                p.update_presentation(cx);
                assert_eq!(p.live_rules.len(), 1);
                p.live = false;
                p.update_presentation(cx);
                assert!(p.live_rules.is_empty());
            })
            .unwrap();
    }
}
