use super::*;
use image::ImageEncoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "[u16; 4]")]
pub(super) struct Area([u16; 4]);

impl TryFrom<[u16; 4]> for Area {
    type Error = &'static str;
    fn try_from(edges: [u16; 4]) -> Result<Self, Self::Error> {
        let [l, t, r, b] = edges;
        if l >= r || t >= b {
            return Err("无效的区域坐标");
        }
        Ok(Self(edges))
    }
}

impl Area {
    pub(super) fn between(
        a: Point<Pixels>,
        b: Point<Pixels>,
        rect: Bounds<Pixels>,
    ) -> Option<Self> {
        if rect.size.width <= px(0.) || rect.size.height <= px(0.) {
            return None;
        }
        let unit = |v: Pixels, extent: Pixels| {
            ((v / extent).clamp(0., 1.) * f32::from(u16::MAX)).round() as u16
        };
        let result = Self::try_from([
            unit(a.x.min(b.x) - rect.left(), rect.size.width),
            unit(a.y.min(b.y) - rect.top(), rect.size.height),
            unit(a.x.max(b.x) - rect.left(), rect.size.width),
            unit(a.y.max(b.y) - rect.top(), rect.size.height),
        ])
        .ok()?;
        let bounds = result.bounds(rect);
        (bounds.size.width >= px(4.) && bounds.size.height >= px(4.)).then_some(result)
    }

    pub(super) fn bounds(self, rect: Bounds<Pixels>) -> Bounds<Pixels> {
        let [l, t, r, b] = self.0.map(|edge| f32::from(edge) / f32::from(u16::MAX));
        Bounds::new(
            rect.origin + point(rect.size.width * l, rect.size.height * t),
            size(rect.size.width * (r - l), rect.size.height * (b - t)),
        )
    }

    pub(super) fn fragment(self) -> String {
        let [l, t, r, b] = self.0;
        format!("area={l}-{t}-{r}-{b}")
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split('-');
        let mut edges = [0; 4];
        for edge in &mut edges {
            *edge = fields.next()?.parse().ok()?;
        }
        if fields.next().is_some() {
            return None;
        }
        edges.try_into().ok()
    }

    pub(super) fn png(self, image: &RenderImage) -> anyhow::Result<Vec<u8>> {
        let dimensions = image.size(0);
        let width = i32::from(dimensions.width) as usize;
        let height = i32::from(dimensions.height) as usize;
        let data = image
            .as_bytes(0)
            .ok_or_else(|| anyhow::anyhow!("页面尚未加载"))?;
        let [l, t, r, b] = self.0.map(usize::from);
        let scale = usize::from(u16::MAX);
        let (x, y) = (l * width / scale, t * height / scale);
        let (end_x, end_y) = ((r * width).div_ceil(scale), (b * height).div_ceil(scale));
        let mut pixels = Vec::with_capacity((end_x - x) * (end_y - y) * 4);
        for row in y..end_y {
            for bgra in data[(row * width + x) * 4..(row * width + end_x) * 4].chunks_exact(4) {
                pixels.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
            }
        }
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded).write_image(
            &pixels,
            (end_x - x) as u32,
            (end_y - y) as u32,
            image::ExtendedColorType::Rgba8,
        )?;
        Ok(encoded)
    }
}

impl Reader {
    pub(super) fn finish_area(
        &mut self,
        end: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.area_dragging = false;
        self.area_preview = None;
        if let Some(area) = Area::between(self.press, end, bounds) {
            self.save_mark(Some(Style::Area(area)), window, cx);
        } else if let Some(index) = self.area_at(end, bounds) {
            self.edit_mark(index, window, cx);
        }
        cx.notify();
    }

    pub(super) fn area_at(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<usize> {
        self.annotations.iter().rposition(|m| {
            m.page == self.selected_page
                && matches!(m.style, Style::Area(area) if area.bounds(bounds).contains(&position))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn pdf_area_crops_only_the_selected_pixels_in_rgb_order() {
        let pixels =
            image::RgbaImage::from_raw(3, 1, vec![0, 0, 255, 255, 100, 20, 3, 255, 255, 0, 0, 255])
                .unwrap();
        let frame = RenderImage::new(vec![image::Frame::new(pixels)]);
        let area = Area::try_from([21845, 0, 43690, 65535]).unwrap();
        let png = image::load_from_memory(&area.png(&frame).unwrap())
            .unwrap()
            .into_rgba8();
        assert_eq!(png.dimensions(), (1, 1));
        assert_eq!(png.into_raw(), [3, 20, 100, 255]);
        assert!(serde_json::from_str::<Area>("[3,0,2,19]").is_err());
    }
}
