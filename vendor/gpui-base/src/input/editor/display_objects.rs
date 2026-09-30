//! Transient source-backed presentation objects, separate from editable tokens.
use crate::input::{EditorMode, InputBaseState, LineTypography, RopeExt};
use gpui::{Bounds, Context, Pixels, Size, point, px};
use std::{ops::Range, rc::Rc};
use sum_tree::Bias;

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayObject {
    pub id: u64,
    pub source: Range<usize>,
    pub size: Size<Pixels>,
    pub baseline: Option<Pixels>,
}

#[derive(Default)]
pub struct DisplayProjection {
    pub replacements: Vec<(Range<usize>, Pixels)>,
    pub hidden_lines: Vec<usize>,
    pub typography: Vec<LineTypography>,
}

#[derive(Clone, Copy)]
pub struct DisplayScrollAnchor {
    offset: usize,
    y: Pixels,
    revision: u64,
}
impl DisplayScrollAnchor {
    pub fn source_offset(&self) -> usize {
        self.offset
    }
    pub fn viewport_y(&self) -> Pixels {
        self.y
    }
}

impl InputBaseState<EditorMode> {
    pub fn display_scroll_anchor(&self) -> Option<DisplayScrollAnchor> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let (offset, _) =
            self.index_for_mouse_position(self.input_bounds().origin + point(px(1.), px(1.)));
        let p = self.text.offset_to_point(offset);
        let row = self
            .display_map
            .buffer_pos_to_display_pos(crate::input::BufferPoint::new(p.row, p.column))
            .row;
        Some(DisplayScrollAnchor {
            offset,
            y: bounds.top() + self.display_map.row_top(row, layout.line_height),
            revision: self.document_revision,
        })
    }
    pub fn restore_display_scroll_anchor(
        &mut self,
        anchor: DisplayScrollAnchor,
        cx: &mut Context<Self>,
    ) -> bool {
        if anchor.revision != self.document_revision {
            return false;
        }
        let (Some(bounds), Some(layout)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return false;
        };
        let p = self.text.offset_to_point(anchor.offset);
        let row = self
            .display_map
            .buffer_pos_to_display_pos(crate::input::BufferPoint::new(p.row, p.column))
            .row;
        let current = bounds.top() + self.display_map.row_top(row, layout.line_height);
        self.set_scroll_offset(
            point(
                self.scroll_offset().x,
                self.scroll_offset().y + anchor.y - current,
            ),
            cx,
        );
        true
    }
    /// Install metadata for the exact current source snapshot. Invalid or
    /// overlapping objects are ignored. This never modifies content/history.
    /// The host composes `display_projection` with its other display layers.
    pub fn set_display_objects(
        &mut self,
        source: &str,
        mut objects: Vec<DisplayObject>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ime_marked_range.is_some() || self.value().as_ref() != source {
            return false;
        }
        objects.retain(|o| {
            let r = &o.source;
            r.start < r.end
                && r.end <= self.text.len()
                && self.text.clip_offset(r.start, Bias::Left) == r.start
                && self.text.clip_offset(r.end, Bias::Left) == r.end
                && crate::input::grapheme_cursor::snap_grapheme(&self.text, r.start, false)
                    == r.start
                && crate::input::grapheme_cursor::snap_grapheme(&self.text, r.end, false) == r.end
                && [o.size.width, o.size.height]
                    .iter()
                    .all(|v| f32::from(*v).is_finite() && *v > px(0.) && *v <= px(8192.))
                && o.baseline
                    .is_none_or(|b| f32::from(b).is_finite() && b >= px(0.) && b <= o.size.height)
        });
        objects.sort_by_key(|o| (o.source.start, std::cmp::Reverse(o.source.end)));
        let mut end = 0;
        objects.retain(|o| {
            if o.source.start < end {
                false
            } else {
                end = o.source.end;
                true
            }
        });
        if self.display_objects.as_ref() == objects.as_slice() {
            return false;
        }
        self.display_objects = objects.into();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        cx.notify();
        true
    }
    pub fn display_objects(&self) -> &[DisplayObject] {
        &self.display_objects
    }

    /// Project the first source line to an atomic width, suppress subsequent
    /// source rows, and reserve measured height in the existing pixel map.
    pub fn display_projection(
        &self,
        base_height: Pixels,
        mut typography: Vec<LineTypography>,
    ) -> DisplayProjection {
        let mut projection = DisplayProjection::default();
        for object in self.display_objects.iter() {
            let raw = self.text.slice(object.source.clone()).to_string();
            let first_end = object.source.start + raw.find(['\r', '\n']).unwrap_or(raw.len());
            if first_end <= object.source.start {
                continue;
            }
            let anchor = object.source.start..first_end;
            projection
                .replacements
                .push((anchor.clone(), object.size.width));
            for (offset, _) in raw.match_indices('\n') {
                let start = object.source.start + offset + 1;
                if start < object.source.end {
                    projection.hidden_lines.push(start);
                }
            }
            let row = self.text.offset_to_point(anchor.start).row;
            let scale =
                (f32::from(object.size.height) / f32::from(base_height).max(1.)).clamp(1., 512.);
            if let Some(existing) = typography
                .iter_mut()
                .find(|s| self.text.offset_to_point(s.anchor.start).row == row)
            {
                existing.height_scale = existing.height_scale.max(scale);
            } else {
                typography.push(LineTypography::new(anchor, 1., scale));
            }
        }
        projection.typography = typography;
        projection
    }

    /// Bounds reflect scrolling, soft wrapping, row heights and fold visibility.
    pub fn display_object_bounds(&self, id: u64) -> Option<Bounds<Pixels>> {
        let object = self.display_objects.iter().find(|o| o.id == id)?;
        let raw = self.text.slice(object.source.clone()).to_string();
        let end = object.source.start + raw.find(['\r', '\n']).unwrap_or(raw.len());
        let mut bounds = self.range_to_bounds(&(object.source.start..end))?;
        bounds.size = object.size;
        Some(bounds)
    }

    pub(crate) fn clear_display_objects(&mut self) {
        self.display_objects = Rc::from([]);
    }
}
