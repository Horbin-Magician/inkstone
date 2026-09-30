//! Display-only zero-width spans. Source offsets, clipboard and undo stay unchanged.
use crate::input::{EditorMode, InputBaseState, RopeExt};
use gpui::{Context, Pixels, px};
use std::ops::Range;
use sum_tree::Bias;

impl InputBaseState<EditorMode> {
    /// Hide complete source lines without recording user folds or changing text.
    /// Anchors must be line starts after the first line. Text edits clear them.
    pub fn set_concealed_lines(&mut self, anchors: Vec<usize>, cx: &mut Context<Self>) -> bool {
        let mut lines: Vec<_> = anchors
            .into_iter()
            .filter_map(|offset| {
                if offset >= self.text.len() || self.text.clip_offset(offset, Bias::Left) != offset
                {
                    return None;
                }
                let point = self.text.offset_to_point(offset);
                (point.row > 0 && point.column == 0).then_some(point.row)
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        if !self.display_map.set_concealed_lines(lines) {
            return false;
        }
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        cx.notify();
        true
    }

    pub fn concealed_lines(&self) -> &[usize] {
        self.display_map.concealed_lines()
    }

    /// Set source ranges omitted from painting and wrapping. Invalid ranges are ignored.
    /// Only whole graphemes within one logical line can be concealed.
    /// Edits touching a range discard it; other ranges follow the edit until replaced.
    pub fn set_concealed_ranges(
        &mut self,
        ranges: Vec<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.set_concealed_ranges_with_widths(
            ranges.into_iter().map(|range| (range, px(0.))).collect(),
            cx,
        )
    }

    /// Reserve display width for source-backed controls. Overlapping replacements
    /// are ignored; adjacent zero-width ranges can still be merged.
    pub fn set_concealed_ranges_with_widths(
        &mut self,
        mut ranges: Vec<(Range<usize>, Pixels)>,
        cx: &mut Context<Self>,
    ) -> bool {
        ranges.sort_by_key(|(r, _)| (r.start, r.end));
        if ranges.as_slice() == self.concealment.as_ref() {
            return false;
        }
        ranges.retain(|(r, width)| {
            f32::from(*width).is_finite()
                && *width >= px(0.)
                && r.start < r.end
                && r.end <= self.text.len()
                && self.text.clip_offset(r.start, Bias::Left) == r.start
                && self.text.clip_offset(r.end, Bias::Left) == r.end
                && crate::input::grapheme_cursor::snap_grapheme(&self.text, r.start, false)
                    == r.start
                && crate::input::grapheme_cursor::snap_grapheme(&self.text, r.end, false) == r.end
                && !self
                    .text
                    .slice(r.clone())
                    .chars()
                    .any(|c| matches!(c, '\r' | '\n'))
        });
        let mut merged: Vec<(Range<usize>, Pixels)> = vec![];
        for (range, width) in ranges {
            if let Some((last, last_width)) = merged.last_mut()
                && range.start <= last.end
                && width == px(0.)
                && *last_width == px(0.)
            {
                last.end = last.end.max(range.end);
            } else if merged
                .last()
                .is_none_or(|(last, _)| range.start >= last.end)
            {
                merged.push((range, width));
            }
        }
        if self.concealment.as_ref() == merged.as_slice() {
            return false;
        }
        self.concealment = merged.into();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.longest_line_width.set(None);
        cx.notify();
        true
    }
    /// The current presentation spans, in source bytes.
    pub fn concealed_ranges(&self) -> Vec<Range<usize>> {
        self.concealment
            .iter()
            .map(|(range, _)| range.clone())
            .collect()
    }
}
