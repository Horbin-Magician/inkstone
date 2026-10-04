//! Reading state restoration, focus, and source-position navigation.

use super::*;

impl EditorPane {
    pub fn callout_states(&self, cx: &App) -> std::collections::BTreeMap<u64, bool> {
        self.preview.read(cx).callout_states()
    }
    pub fn restore_callout_states(
        &mut self,
        states: &std::collections::BTreeMap<u64, bool>,
        cx: &mut Context<Self>,
    ) {
        self.preview
            .update(cx, |preview, cx| preview.restore_callout_states(states, cx));
    }
    pub fn reading_position(&self, cx: &App) -> inkstone::preferences::ReadingPosition {
        self.pending_reading_position.unwrap_or_else(|| {
            let position = self.preview.read(cx).scroll_position();
            inkstone::preferences::ReadingPosition {
                block: position.item_ix,
                offset: f32::from(position.offset_in_item),
            }
        })
    }
    pub fn restore_reading_position(
        &mut self,
        position: inkstone::preferences::ReadingPosition,
        cx: &mut Context<Self>,
    ) {
        self.pending_preview_jump = None;
        self.pending_reading_position = Some(position);
        cx.notify();
    }
    pub fn reading_bounds(&self, cx: &App) -> Bounds<Pixels> {
        self.preview.read(cx).bounds()
    }
    pub fn focus_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            let handle = self.preview.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }
    pub fn jump(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_reading_position = None;
        if self.reading {
            self.pending_preview_jump = Some(offset);
            self.focus_view(window, cx);
            cx.notify();
            return;
        }
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
}
