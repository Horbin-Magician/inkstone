//! Glyph bounds in normalized bitmap coordinates, independent of GPUI zoom and scroll.
use gpui::{Bounds, Pixels, Point, point, px, size};
use pdfium_render::prelude::{PdfPage, PdfRenderConfig};
use std::ops::Range;

struct Glyph {
    bytes: Range<usize>,
    rect: Option<[f32; 4]>,
}

#[derive(Default)]
pub(super) struct TextLayer {
    pub(super) text: String,
    glyphs: Vec<Glyph>,
}

impl TextLayer {
    pub(super) fn read(page: &PdfPage<'_>, config: &PdfRenderConfig, w: i32, h: i32) -> Self {
        let mut layer = Self::default();
        let Ok(text) = page.text() else { return layer };
        for ch in text.chars().iter() {
            let Some(c) = ch.unicode_char() else { continue };
            let start = layer.text.len();
            layer.text.push(c);
            let rect = ch
                .loose_bounds()
                .or_else(|_| ch.tight_bounds())
                .ok()
                .and_then(|r| {
                    let (x1, y1) = page.points_to_pixels(r.left(), r.top(), config).ok()?;
                    let (x2, y2) = page.points_to_pixels(r.right(), r.bottom(), config).ok()?;
                    Some([
                        x1.min(x2) as f32 / w as f32,
                        y1.min(y2) as f32 / h as f32,
                        x1.max(x2) as f32 / w as f32,
                        y1.max(y2) as f32 / h as f32,
                    ])
                })
                .filter(|r| r[2] > r[0] && r[3] > r[1]);
            layer.glyphs.push(Glyph {
                bytes: start..layer.text.len(),
                rect,
            });
        }
        layer
    }

    pub(super) fn hit(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        nearest: bool,
    ) -> Option<Range<usize>> {
        let x = f32::from(position.x - bounds.origin.x) / f32::from(bounds.size.width);
        let y = f32::from(position.y - bounds.origin.y) / f32::from(bounds.size.height);
        let mut best = None;
        let mut distance = f32::INFINITY;
        for glyph in &self.glyphs {
            let Some([l, t, r, b]) = glyph.rect else {
                continue;
            };
            let dx = (x - x.clamp(l, r)) * f32::from(bounds.size.width);
            let dy = (y - y.clamp(t, b)) * f32::from(bounds.size.height);
            let d = dx * dx + dy * dy;
            if d == 0. {
                return Some(glyph.bytes.clone());
            }
            if nearest && d < distance {
                distance = d;
                best = Some(glyph.bytes.clone());
            }
        }
        best
    }

    pub(super) fn boxes(
        &self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
    ) -> impl Iterator<Item = Bounds<Pixels>> + '_ {
        let start = self.glyphs.partition_point(|g| g.bytes.end <= range.start);
        self.glyphs[start..]
            .iter()
            .take_while(move |g| g.bytes.start < range.end)
            .filter_map(move |g| {
                let [l, t, r, b] = g.rect?;
                Some(Bounds::new(
                    bounds.origin + point(bounds.size.width * l, bounds.size.height * t),
                    size(
                        (bounds.size.width * (r - l)).max(px(1.)),
                        (bounds.size.height * (b - t)).max(px(1.)),
                    ),
                ))
            })
    }
}
