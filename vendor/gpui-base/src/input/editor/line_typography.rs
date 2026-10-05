//! Source-anchored typography for individual editor lines.
use crate::input::{EditorMode, InputBaseState, RopeExt};
use gpui::{Context, Pixels, px};
use ropey::Rope;
use std::{ops::Range, rc::Rc};
use sum_tree::Bias;

#[derive(Clone, Debug, PartialEq)]
pub struct LineTypography {
    pub(crate) anchor: Range<usize>,
    pub(crate) font_scale: f32,
    pub(crate) height_scale: f32,
    pub(crate) left_padding: Pixels,
}
impl LineTypography {
    /// Anchor a line's font and row height to a stable source prefix.
    pub fn new(anchor: Range<usize>, font_scale: f32, height_scale: f32) -> Self {
        Self {
            anchor,
            font_scale,
            height_scale,
            left_padding: px(0.),
        }
    }
    /// Inset every visual row, including soft wraps, without changing source offsets.
    pub fn with_left_padding(mut self, padding: Pixels) -> Self {
        self.left_padding = padding;
        self
    }

    pub fn source_anchor(&self) -> &Range<usize> {
        &self.anchor
    }
}
fn valid(text: &Rope, style: &LineTypography) -> bool {
    let r = &style.anchor;
    r.start < r.end
        && r.end <= text.len()
        && f32::from(style.left_padding).is_finite()
        && style.left_padding >= px(0.)
        && style.font_scale.is_finite()
        && style.height_scale.is_finite()
        && (0.25..=4.).contains(&style.font_scale)
        && (0.25..=512.).contains(&style.height_scale)
        && text.clip_offset(r.start, Bias::Left) == r.start
        && text.clip_offset(r.end, Bias::Left) == r.end
        && !text
            .slice(r.clone())
            .chars()
            .any(|c| matches!(c, '\r' | '\n'))
}
pub(crate) fn rebase(
    styles: &Rc<[LineTypography]>,
    text: &Rope,
    range: &Range<usize>,
    len: usize,
) -> Rc<[LineTypography]> {
    let delta = len as isize - range.len() as isize;
    let mut next: Vec<_> = styles
        .iter()
        .filter_map(|s| {
            if s.anchor.start <= range.end && range.start < s.anchor.end {
                return None;
            }
            let mut s = s.clone();
            if s.anchor.start >= range.end {
                s.anchor = s.anchor.start.checked_add_signed(delta)?
                    ..s.anchor.end.checked_add_signed(delta)?;
            }
            valid(text, &s).then_some(s)
        })
        .collect();
    next.dedup_by_key(|s| text.offset_to_point(s.anchor.start).row);
    next.into()
}
pub(crate) fn for_line<'a>(
    styles: &'a [LineTypography],
    text: &Rope,
    row: usize,
) -> Option<&'a LineTypography> {
    let start = text.line_start_offset(row);
    let end = text.line_end_offset(row);
    styles
        .get(styles.partition_point(|s| s.anchor.start < start))
        .filter(|s| s.anchor.start < end)
}
impl InputBaseState<EditorMode> {
    pub fn set_line_typography(
        &mut self,
        mut styles: Vec<LineTypography>,
        cx: &mut Context<Self>,
    ) -> bool {
        styles.retain(|s| valid(&self.text, s));
        styles.sort_by_key(|s| s.anchor.start);
        styles.dedup_by_key(|s| self.text.offset_to_point(s.anchor.start).row);
        if self.line_typography.as_ref() == styles.as_slice() {
            return false;
        }
        self.line_typography = styles.into();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.display_map
            .set_line_typography(self.line_typography.clone(), cx);
        self.longest_line_width.set(None);
        cx.notify();
        true
    }
}
