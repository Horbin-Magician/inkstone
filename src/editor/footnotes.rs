use super::*;
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use std::ops::Range;

pub(super) struct FootnoteEdit {
    input: Entity<TextareaState>,
    baseline: SharedString,
    body: Range<usize>,
    continuation: String,
    anchor: Point<Pixels>,
    error: bool,
    synced_input: SharedString,
    max_rows: usize,
    _subscription: Subscription,
}

impl EditorPane {
    pub fn footnote_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.footnote_edit.as_ref().is_some_and(|edit| {
            edit.error
                || edit.input.read(cx).value() != edit.synced_input
                || edit
                    .input
                    .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        })
    }
    pub fn has_footnote_editor(&self) -> bool {
        self.footnote_edit.is_some()
    }

    pub fn open_new_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_footnote(window, cx);
    }

    pub fn open_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.footnote_edit.is_some() {
            return false;
        }
        let source = self.editor.read(cx).value();
        let cursor = self.editor.read(cx).selected_range().start;
        let Some((body, text, continuation)) = footnote_body(&source, cursor) else {
            return false;
        };
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(text.clone())
                .auto_grow(1, 16)
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.sync_footnote(window, cx);
            }
        });
        self.footnote_edit = Some(FootnoteEdit {
            input: input.clone(),
            baseline: source,
            body,
            continuation,
            anchor: self.editor.read(cx).input_bounds().origin,
            error: false,
            synced_input: text.into(),
            max_rows: 16,
            _subscription: subscription,
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        // Wait for the insertion's layout so the popup follows the new reference.
        cx.on_next_frame(window, |this, _, cx| {
            let state = this.editor.read(cx);
            let anchor = state
                .cursor_layout()
                .map(|(caret, _)| point(caret.left(), caret.bottom()) + state.scroll_offset());
            if let Some(edit) = &mut this.footnote_edit {
                if let Some(anchor) = anchor {
                    edit.anchor = anchor;
                }
                cx.notify();
            }
        });
        cx.notify();
        true
    }

    fn sync_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.footnote_edit else {
            return;
        };
        if edit
            .input
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        {
            return;
        }
        let input = edit.input.read(cx).value();
        if input == edit.synced_input {
            edit.error = false;
            return;
        }
        let source = self.editor.read(cx).value();
        if source != edit.baseline {
            edit.error = true;
            cx.notify();
            return;
        }
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let text = edit.input.read(cx).value().replace("\r\n", "\n");
        let replacement = text.replace('\n', &format!("{newline}{}", edit.continuation));
        if source[edit.body.clone()] == replacement {
            edit.synced_input = input;
            edit.error = false;
            return;
        }
        let selection = self.editor.read(cx).selected_range();
        let map = |offset: usize| {
            if offset >= edit.body.end {
                offset.saturating_add_signed(replacement.len() as isize - edit.body.len() as isize)
            } else if offset > edit.body.start {
                edit.body.start
            } else {
                offset
            }
        };
        let selection = map(selection.start)..map(selection.end);
        let applied = self.editor.update(cx, |s, cx| {
            s.apply_source_edit(edit.body.clone(), &replacement, selection, window, cx)
        });
        if applied {
            edit.body.end = edit.body.start + replacement.len();
            edit.baseline = self.editor.read(cx).value();
            edit.synced_input = input;
            edit.error = false;
        } else {
            edit.error = true;
        }
        cx.notify();
    }

    pub fn close_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_footnote(window, cx);
        let Some(edit) = &self.footnote_edit else {
            return;
        };
        if edit.error
            || edit
                .input
                .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        {
            return;
        }
        self.footnote_edit = None;
        self.editor.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub(super) fn footnote_panel(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let edit = self.footnote_edit.as_mut()?;
        let line_height = self.font_size * 0.875 * 1.5;
        let height_limit = f32::from(window.viewport_size().height * 0.8).min(400.);
        let max_rows = ((height_limit - 26.).max(line_height) / line_height).floor() as usize;
        if edit.max_rows != max_rows {
            edit.max_rows = max_rows;
            edit.input
                .update(cx, |s, cx| s.set_auto_grow(1, max_rows, cx));
        }
        let panel = div()
            .id("footnote-editor")
            .occlude()
            .w(px(450.).min(window.viewport_size().width * 0.8))
            .max_h(px(height_limit))
            .overflow_y_scroll()
            .py(px(12.))
            .px(px(16.))
            .rounded_md()
            .shadow_lg()
            .border_1()
            .border_color(rgb(if self.light { 0xdddddd } else { 0x454545 }))
            .bg(crate::theme::palette(self.light).background)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_footnote(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_footnote(window, cx)))
            .child(
                Textarea::new(&edit.input)
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(self.font_size * 0.875))
                    .line_height(relative(1.5))
                    .font_family("Microsoft YaHei UI"),
            )
            .when(edit.error, |s| {
                s.child("笔记已变化，无法自动写回。请保留此处内容并复制到笔记。")
                    .child(
                        gpui_component::button::Button::new("discard-footnote-draft")
                            .label("放弃此处修改")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.footnote_edit = None;
                                this.editor.update(cx, |s, cx| s.focus(window, cx));
                                cx.notify();
                            })),
                    )
            });
        Some(deferred(anchored().position(edit.anchor).child(panel)).into_any_element())
    }
}

fn footnote_body(source: &str, cursor: usize) -> Option<(Range<usize>, String, String)> {
    let parsed = index::parse(source);
    if let Some(note) = parsed
        .inline_footnotes
        .iter()
        .find(|n| n.range.start <= cursor && cursor <= n.range.end)
    {
        return Some((
            note.content.clone(),
            source[note.content.clone()].to_owned(),
            String::new(),
        ));
    }
    let (_, id) = parsed
        .footnotes
        .iter()
        .find(|(range, _)| range.start.saturating_sub(2) <= cursor && cursor <= range.end + 1)?;
    let (definition, _) = parsed
        .footnote_definitions
        .iter()
        .find(|(_, name)| name == id)?;
    let (label, _) = parsed
        .footnotes
        .iter()
        .find(|(range, name)| range.start == definition.start + 2 && name == id)?;
    let line_start = source[..definition.start].rfind('\n').map_or(0, |n| n + 1);
    let prefix: String = source[line_start..definition.start]
        .chars()
        .map(|c| {
            if matches!(c, '>' | ' ' | '\t') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let continuation = format!("{prefix}    ");
    let mut start = label.end + 2;
    while matches!(source.as_bytes().get(start), Some(b' ' | b'\t')) {
        start += 1;
    }
    let mut end = definition.end.max(start);
    while end > start && matches!(source.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    let body = &source[start..end];
    let normalized = body.replace("\r\n", "\n");
    let text = normalized
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line
            } else {
                line.strip_prefix(&continuation)
                    .or_else(|| {
                        line.strip_prefix(&prefix)
                            .and_then(|s| s.strip_prefix('\t'))
                    })
                    .or_else(|| line.strip_prefix(&prefix).filter(|s| s.trim().is_empty()))
                    .or_else(|| line.strip_prefix('\t'))
                    .unwrap_or(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some((start..end, text, continuation))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn inline_footnote_popup_writes_only_body_and_undo_restores_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "正文^[**说明**] 后续\r\n";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                let start = source.find("^[").unwrap();
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(start..start, cx));
                assert!(p.open_footnote(w, cx));
                let input = p.footnote_edit.as_ref().unwrap().input.clone();
                input.update(cx, |s, cx| s.set_value("新说明😀", w, cx));
                p.sync_footnote(w, cx);
                assert_eq!(
                    p.editor.read(cx).value().as_ref(),
                    "正文^[新说明😀] 后续\r\n"
                );
                p.close_footnote(w, cx);
                p.editor
                    .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
    use core::prelude::v1::test;
    #[gpui::test]
    fn footnote_popup_grows_caps_and_shrinks_with_content(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("[^1]\n\n[^1]: x\n", w, cx));
        let input = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
                p.open_footnote(w, cx);
                p.footnote_edit.as_ref().unwrap().input.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..3 {
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let small = input.read_with(&visual, |s, _| s.input_bounds().size.height);
        assert!(small > px(0.) && small < px(80.), "{small:?}");
        handle
            .update(&mut visual, |p, w, cx| {
                input.update(cx, |s, cx| s.set_value("long content\n".repeat(80), w, cx));
                p.sync_footnote(w, cx);
            })
            .unwrap();
        for _ in 0..3 {
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let large = input.read_with(&visual, |s, _| s.input_bounds().size.height);
        assert!(
            large > small && large <= px(400.),
            "small={small:?}, large={large:?}"
        );
        visual.simulate_resize(size(px(320.), px(220.)));
        for _ in 0..3 {
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        input.read_with(&visual, |s, _| {
            let bounds = s.input_bounds();
            assert!(bounds.size.height < large);
            assert!(
                bounds.left() >= px(0.) && bounds.right() <= px(320.),
                "{bounds:?}"
            );
            assert!(
                bounds.top() >= px(0.) && bounds.bottom() <= px(220.),
                "{bounds:?}"
            );
        });
        handle
            .update(&mut visual, |p, w, cx| {
                input.update(cx, |s, cx| s.set_value("短", w, cx));
                p.sync_footnote(w, cx);
            })
            .unwrap();
        for _ in 0..3 {
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        visual.update(|w, cx| {
            let state = input.read(cx);
            assert!(state.input_bounds().size.height < large);
            assert!(state.focus_handle(cx).is_focused(w));
            assert_eq!(state.scroll_offset().y, px(0.));
        });
    }
    #[gpui::test]
    fn existing_footnote_edit_preserves_following_text_and_reference_position(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let original = "[^A]: first\r\n\r\n    second\r\n\r\nafter 中文[^a] end";
        let handle = cx.add_window(|w, cx| EditorPane::new(original, w, cx));
        handle
            .update(cx, |p, w, cx| {
                let cursor = original.find("[^a]").unwrap() + 2;
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(cursor..cursor, cx));
                assert!(p.open_footnote(w, cx));
                let input = p.footnote_edit.as_ref().unwrap().input.clone();
                assert_eq!(input.read(cx).value().as_ref(), "first\n\nsecond");
                input.update(cx, |s, cx| s.set_value("短内容\n\n- item", w, cx));
                p.sync_footnote(w, cx);
                let expected = "[^A]: 短内容\r\n    \r\n    - item\r\n\r\nafter 中文[^a] end";
                assert_eq!(p.editor.read(cx).value().as_ref(), expected);
                assert_eq!(
                    p.editor.read(cx).selected_range().start,
                    expected.find("[^a]").unwrap() + 2
                );
                p.close_footnote(w, cx);
                assert!(p.open_footnote(w, cx));
                assert_eq!(
                    p.footnote_edit
                        .as_ref()
                        .unwrap()
                        .input
                        .read(cx)
                        .value()
                        .as_ref(),
                    "短内容\n\n- item"
                );
            })
            .unwrap();
        assert!(footnote_body("`[^a]`\n\n[^a]: text", 3).is_none());
        let quoted = "ref[^q]\n\n> [^q]: first\n>     second\n\nend";
        let (_, text, continuation) = footnote_body(quoted, 5).unwrap();
        assert_eq!(text, "first\nsecond");
        assert_eq!(continuation, ">     ");
    }

    #[gpui::test]
    fn footnote_popup_writes_multiline_undoes_and_returns_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("文[^1]\r\n\r\n[^1]: \r\n", w, cx));
        let (source, input) = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(7..7, cx));
                p.open_new_footnote(w, cx);
                (
                    p.editor.clone(),
                    p.footnote_edit.as_ref().unwrap().input.clone(),
                )
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.update(|w, cx| assert!(input.read(cx).focus_handle(cx).is_focused(w)));
        handle
            .update(&mut visual, |p, w, cx| {
                input.update(cx, |s, cx| {
                    s.apply_source_edit(0..0, "中文😀\nsecond", 0..0, w, cx);
                });
                p.sync_footnote(w, cx);
                assert_eq!(
                    source.read(cx).value().as_ref(),
                    "文[^1]\r\n\r\n[^1]: 中文😀\r\n    second\r\n"
                );
                assert_eq!(source.read(cx).selected_range(), 7..7);
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |p, w, cx| {
                p.sync_footnote(w, cx);
                assert_eq!(source.read(cx).value().as_ref(), "文[^1]\r\n\r\n[^1]: \r\n");
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        handle
            .update(&mut visual, |p, w, cx| {
                assert!(!p.has_footnote_editor());
                assert!(source.read(cx).focus_handle(cx).is_focused(w));
            })
            .unwrap();
    }

    #[gpui::test]
    fn footnote_popup_preserves_draft_on_source_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("[^1]\n\n[^1]: \n", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(4..4, cx));
                p.open_new_footnote(w, cx);
                let input = p.footnote_edit.as_ref().unwrap().input.clone();
                input.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
                });
                p.close_footnote(w, cx);
                assert!(p.footnote_pending(w, cx));
                assert_eq!(p.editor.read(cx).value().as_ref(), "[^1]\n\n[^1]: \n");
                input.update(cx, |s, cx| s.replace_text_in_range(None, "你", w, cx));
                p.sync_footnote(w, cx);
                assert_eq!(p.editor.read(cx).value().as_ref(), "[^1]\n\n[^1]: 你\n");
                assert!(!p.footnote_pending(w, cx));
                input.update(cx, |s, cx| s.set_value("draft", w, cx));
                p.editor.update(cx, |s, cx| s.set_value("external", w, cx));
                p.close_footnote(w, cx);
                assert!(p.footnote_pending(w, cx));
                assert!(p.footnote_edit.as_ref().unwrap().error);
                assert_eq!(input.read(cx).value().as_ref(), "draft");
                assert_eq!(p.editor.read(cx).value().as_ref(), "external");
            })
            .unwrap();
    }
}
