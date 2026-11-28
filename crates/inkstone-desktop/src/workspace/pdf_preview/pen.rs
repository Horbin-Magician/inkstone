use super::*;
use std::fmt::Write as _;

pub(super) type Sample = [u16; 2];
const WIDTH: f32 = 0.003;

pub(super) fn sample(pos: Point<Pixels>, bounds: Bounds<Pixels>) -> Sample {
    let local = pos - bounds.origin;
    [
        (local.x / bounds.size.width).clamp(0., 1.),
        (local.y / bounds.size.height).clamp(0., 1.),
    ]
    .map(|n| (n * f32::from(u16::MAX)).round() as u16)
}

fn screen(p: Sample, bounds: Bounds<Pixels>) -> Point<Pixels> {
    let [x, y] = p.map(|n| f32::from(n) / f32::from(u16::MAX));
    bounds.origin + point(bounds.size.width * x, bounds.size.height * y)
}

pub(super) fn draw_stroke(
    stroke: &[Sample],
    bounds: Bounds<Pixels>,
    color: Color,
    window: &mut Window,
) {
    let Some(&first) = stroke.first() else { return };
    let pen = (bounds.size.width * WIDTH).max(px(0.5));
    let start = screen(first, bounds);
    if stroke.len() == 1 {
        window.paint_quad(quad(
            Bounds::new(start - point(pen * 0.5, pen * 0.5), size(pen, pen)),
            pen * 0.5,
            rgb(color.rgb()),
            px(0.),
            rgb(color.rgb()),
            Default::default(),
        ));
        return;
    }
    let mut path = gpui::PathBuilder::stroke(pen);
    path.move_to(start);
    for p in &stroke[1..] {
        path.line_to(screen(*p, bounds));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(color.rgb()));
    }
}

fn envelope(strokes: &[Vec<Sample>], aspect: f32) -> Option<Area> {
    let mut edges = [u16::MAX, u16::MAX, 0, 0];
    for [x, y] in strokes.iter().flatten().copied() {
        edges = [
            edges[0].min(x),
            edges[1].min(y),
            edges[2].max(x),
            edges[3].max(y),
        ];
    }
    let margin = WIDTH * f32::from(u16::MAX);
    let dx = margin.ceil() as u16;
    let dy = (margin * aspect).ceil() as u16;
    [
        edges[0].saturating_sub(dx),
        edges[1].saturating_sub(dy),
        edges[2].saturating_add(dx),
        edges[3].saturating_add(dy),
    ]
    .try_into()
    .ok()
}

pub(super) fn hit(strokes: &[Vec<Sample>], pos: Point<Pixels>, bounds: Bounds<Pixels>) -> bool {
    let tolerance = (f32::from(bounds.size.width) * WIDTH * 0.5 + 3.).powi(2);
    strokes.iter().any(|stroke| {
        let Some(first) = stroke.first() else {
            return false;
        };
        let mut a = screen(*first, bounds);
        for p in stroke {
            let b = screen(*p, bounds);
            let (dx, dy) = (f32::from(b.x - a.x), f32::from(b.y - a.y));
            let (x, y) = (f32::from(pos.x - a.x), f32::from(pos.y - a.y));
            let length = dx * dx + dy * dy;
            let t = if length > 0. {
                ((x * dx + y * dy) / length).clamp(0., 1.)
            } else {
                0.
            };
            if (x - t * dx).powi(2) + (y - t * dy).powi(2) <= tolerance {
                return true;
            }
            a = b;
        }
        false
    })
}

pub(super) fn svg(strokes: &[Vec<Sample>], area: Area, aspect: f32, color: Color) -> Vec<u8> {
    let bounds = Bounds::new(Point::default(), size(px(1000.), px(1000. / aspect)));
    let rect = area.bounds(bounds);
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}" height="{}"><g fill="none" stroke="#{:06x}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round">"##,
        f32::from(rect.left()),
        f32::from(rect.top()),
        f32::from(rect.size.width),
        f32::from(rect.size.height),
        f32::from(rect.size.width).ceil(),
        f32::from(rect.size.height).ceil(),
        color.rgb()
    );
    for stroke in strokes {
        svg.push_str("<path d=\"");
        for (i, p) in stroke.iter().enumerate() {
            let pos = screen(*p, bounds);
            let _ = write!(
                svg,
                "{}{:.3} {:.3}",
                if i == 0 { 'M' } else { 'L' },
                f32::from(pos.x),
                f32::from(pos.y)
            );
        }
        if stroke.len() == 1 {
            svg.push_str("l0 0");
        }
        svg.push_str("\"/>");
    }
    svg.push_str("</g></svg>");
    svg.into_bytes()
}

impl Reader {
    pub(super) fn push_ink(&mut self, pos: Point<Pixels>, bounds: Bounds<Pixels>) {
        let p = sample(pos, bounds);
        if self.pen_stroke.last() != Some(&p) {
            self.pen_stroke.push(p);
        }
    }
    pub(super) fn finish_ink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.active_page() else {
            self.pen_stroke.clear();
            return;
        };
        let aspect = page.aspect;
        let mut strokes = self
            .editing
            .map(|i| self.annotations[i].strokes.clone())
            .unwrap_or_default();
        strokes.push(std::mem::take(&mut self.pen_stroke));
        let Some(area) = envelope(&strokes, aspect) else {
            return;
        };
        let mark = Mark {
            page: self.selected_page,
            style: Style::Ink(area),
            strokes,
            bytes: 0..0,
            quote: String::new(),
            color: self.color,
            comment: self.comment.read(cx).value().to_string(),
        };
        self.commit_mark(Some(mark), window, cx);
    }
}
