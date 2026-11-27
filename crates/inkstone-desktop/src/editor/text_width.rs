//! Centered text-column resizing. Persist only once the pointer is released.
use super::*;

pub(super) struct Drag {
    start: Pixels,
    width: Pixels,
    left: bool,
}

impl EditorPane {
    pub(super) fn text_width_handle(
        &self,
        left: bool,
        inset: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = if left {
            "text-width-left"
        } else {
            "text-width-right"
        };
        let palette = crate::theme::palette(self.light);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Splitter)
            .aria_label(if left {
                "调整正文宽度（左侧）"
            } else {
                "调整正文宽度（右侧）"
            })
            .absolute()
            .top(relative(0.5))
            .mt(px(-40.))
            .w(px(16.))
            .h(px(80.))
            .when(left, |s| s.left(-inset - px(22.)))
            .when(!left, |s| s.right(-inset - px(22.)))
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .rounded(px(8.))
            .hover(|s| s.bg(palette.surface))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("拖动调整正文宽度，双击恢复默认")
                    .build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    if event.click_count == 2 {
                        this.width_drag = None;
                        this.text_width = 700.;
                        this.readable_width = true;
                        cx.emit(EditorEvent::TextWidthChanged(this.text_width));
                    } else {
                        this.width_drag = Some(Drag {
                            start: event.position.x,
                            width: if this.reading {
                                this.preview.read(cx).bounds().size.width
                            } else {
                                this.editor
                                    .read(cx)
                                    .wrap_width()
                                    .unwrap_or(px(this.text_width))
                            },
                            left,
                        });
                    }
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(3.))
                    .h(px(40.))
                    .rounded_full()
                    .bg(if self.width_drag.is_some() {
                        palette.accent
                    } else {
                        palette.border
                    }),
            )
    }

    fn finish_width_drag(&mut self, cx: &mut Context<Self>) {
        if self.width_drag.take().is_some() {
            if self.readable_width {
                cx.emit(EditorEvent::TextWidthChanged(self.text_width));
            }
            cx.notify();
        }
    }
}

pub(super) fn capture(pane: WeakEntity<EditorPane>) -> impl IntoElement {
    canvas(
        |bounds, _, _| bounds,
        move |_, bounds, window, _| {
            let moving = pane.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let _ = moving.update(cx, |pane, cx| {
                    let Some(drag) = &pane.width_drag else {
                        return;
                    };
                    if event.pressed_button != Some(MouseButton::Left) {
                        pane.finish_width_drag(cx);
                        return;
                    }
                    let delta = (event.position.x - drag.start) * if drag.left { -2. } else { 2. };
                    let maximum = f32::from(bounds.size.width).clamp(240., 2400.);
                    pane.text_width = f32::from(drag.width + delta).clamp(240., maximum);
                    pane.readable_width = true;
                    cx.stop_propagation();
                    cx.notify();
                });
            });
            // Window capture keeps the drag alive beyond the handle and ends it
            // even when the release lands over another pane or outside this one.
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                    return;
                }
                let _ = pane.update(cx, |pane, cx| {
                    if pane.width_drag.is_some() {
                        pane.finish_width_drag(cx);
                        cx.stop_propagation();
                    }
                });
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
    fn text_width_handles_resize_centered_column_and_release_outside(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "正文 content 😀 ".repeat(200);
        let handle = cx.add_window(|w, cx| {
            let mut pane = EditorPane::new(&source, w, cx);
            pane.reading = true;
            pane
        });
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(650.)));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let original = handle
            .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
            .unwrap();
        assert_eq!(original.size.width, px(700.));
        for (id, dx, expected) in [
            ("text-width-left", 50., 600.),
            ("text-width-right", 80., 760.),
            ("text-width-right", -1000., 240.),
            ("text-width-left", -2000., 1096.),
        ] {
            let from = visual.debug_bounds(id).unwrap().center();
            visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
            let to = from + point(px(dx), px(0.));
            visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
            visual.update(|w, cx| w.draw(cx).clear(cx));
            handle
                .update(&mut visual, |p, _, cx| {
                    assert!(p.width_drag.is_none());
                    assert_eq!(p.text_width, expected);
                    let bounds = p.reading_bounds(cx);
                    assert_eq!(bounds.size.width, px(expected));
                    assert_eq!(bounds.center().x, original.center().x);
                    assert_eq!(p.editor.read(cx).value().as_ref(), source);
                })
                .unwrap();
        }
        // Reflow in a narrow split, then restore the user's preferred width.
        for (width, expected) in [(400., 296.), (1200., 1096.)] {
            visual.simulate_resize(size(px(width), px(650.)));
            visual.update(|w, cx| w.draw(cx).clear(cx));
            handle
                .update(&mut visual, |p, _, cx| {
                    assert_eq!(p.reading_bounds(cx).size.width, px(expected));
                    assert_eq!(p.text_width, 1096.);
                })
                .unwrap();
            for id in ["text-width-left", "text-width-right"] {
                let bounds = visual.debug_bounds(id).unwrap();
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
            }
        }
        // Starting with unrestricted width still permits narrowing the column.
        handle
            .update(&mut visual, |p, _, cx| {
                p.readable_width = false;
                cx.notify();
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let from = visual.debug_bounds("text-width-right").unwrap().center();
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        let to = from - point(px(100.), px(0.));
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        handle
            .update(&mut visual, |p, _, _| {
                assert!(p.readable_width);
                assert_eq!(p.text_width, 896.);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let position = visual.debug_bounds("text-width-left").unwrap().center();
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position,
            click_count: 2,
            ..Default::default()
        });
        visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.text_width, 700.);
                assert!(p.width_drag.is_none());
                assert_eq!(p.reading_bounds(cx).size.width, px(700.));
                p.reading = false;
                cx.notify();
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("text-width-left").is_some());
        assert!(visual.debug_bounds("text-width-right").is_some());
    }
    #[gpui::test]
    fn text_width_handles_resize_editing_modes_without_changing_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# 标题\n正文 content 😀\n".repeat(40);
        for (live, width) in [(false, 1200.), (true, 1200.), (false, 440.), (true, 440.)] {
            let handle = cx.add_window(|w, cx| {
                let mut pane = EditorPane::new(&source, w, cx);
                pane.live = live;
                pane.editor.update(cx, |state, cx| {
                    state.set_line_number(true, w, cx);
                    state.set_selected_range(0..8, cx);
                });
                pane
            });
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.simulate_resize(size(px(width), px(650.)));
            visual.run_until_parked();
            for _ in 0..2 {
                visual.update(|w, cx| w.draw(cx).clear(cx));
            }
            for (id, dx) in [("text-width-left", 20.), ("text-width-right", 10.)] {
                let before = handle
                    .update(&mut visual, |p, _, cx| {
                        p.editor.read(cx).wrap_width().unwrap()
                    })
                    .unwrap();
                let bar = visual.debug_bounds(id).unwrap();
                assert!(bar.left() >= px(0.) && bar.right() <= px(width));
                handle
                    .update(&mut visual, |p, _, cx| {
                        let editor_bounds = p.editor.read(cx).text_bounds().unwrap();
                        assert!(
                            !bar.intersects(&editor_bounds),
                            "handle overlaps the editor gutter or text"
                        );
                    })
                    .unwrap();
                let from = bar.center();
                let to = from + point(px(dx), px(0.));
                visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
                visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
                visual.update(|w, cx| w.draw(cx).clear(cx));
                visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
                visual.update(|w, cx| w.draw(cx).clear(cx));
                let delta = if id == "text-width-left" { -40. } else { 20. };
                handle
                    .update(&mut visual, |p, _, cx| {
                        let expected = (f32::from(before) + delta).max(240.);
                        assert!(p.width_drag.is_none());
                        assert_eq!(p.text_width, expected);
                        let state = p.editor.read(cx);
                        assert_eq!(state.wrap_width(), Some(px(expected)));
                        assert_eq!(state.selected_range(), 0..8);
                        assert_eq!(state.value().as_ref(), source);
                    })
                    .unwrap();
            }
        }
    }
}
