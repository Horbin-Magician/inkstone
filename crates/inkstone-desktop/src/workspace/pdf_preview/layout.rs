use super::*;

const GAP: f32 = 8.;

pub(super) struct Layout {
    // Cumulative page heights at unit width; gaps stay constant when zooming.
    heights: Vec<f64>,
}

impl Layout {
    pub(super) fn read(doc: &PdfDocument<'_>) -> anyhow::Result<Self> {
        let mut heights = vec![0.];
        for rect in doc.pages().page_sizes()? {
            let height = f64::from(rect.height().value) / f64::from(rect.width().value);
            anyhow::ensure!(height.is_finite() && height > 0., "无效的页面尺寸");
            heights.push(heights.last().unwrap() + height);
        }
        anyhow::ensure!(heights.len() > 1, "PDF 没有页面");
        Ok(Self { heights })
    }

    pub(super) fn count(&self) -> i32 {
        self.heights.len() as i32 - 1
    }
    pub(super) fn top(&self, page: i32, width: f32) -> f32 {
        (self.heights[page as usize] * f64::from(width)) as f32 + page as f32 * GAP
    }

    fn at(&self, y: f32, width: f32) -> i32 {
        let mut range = 0..self.count();
        while range.start + 1 < range.end {
            let mid = range.start + (range.end - range.start) / 2;
            if self.top(mid, width) <= y {
                range.start = mid;
            } else {
                range.end = mid;
            }
        }
        range.start
    }
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq)]
pub(super) struct Position {
    pub(super) page: i32,
    y: f32,
    x: f32,
    pub(super) zoom: f32,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            page: 0,
            y: 0.,
            x: 0.,
            zoom: 1.,
        }
    }
}

impl Reader {
    pub(super) fn active_page(&self) -> Option<&Arc<Page>> {
        self.pages.get(&self.selected_page)?.as_ref().ok()
    }

    pub(super) fn navigate(&mut self, page: i32) {
        self.page = page;
        self.position.page = page;
        self.position.y = 0.;
        self.selection = None;
        self.anchor = None;
        self.editing = None;
        self.selected_page = -1;
        self.seek = true;
    }

    pub(super) fn persist_position(&mut self) {
        if self.position_dirty {
            match self.store.save_position(&self.position) {
                Ok(()) => self.position_dirty = false,
                Err(e) => self.mark_error = Some(format!("保存阅读位置失败：{e}")),
            }
        }
    }

    pub(super) fn observe_position(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        let width = self.width * self.zoom;
        if self.seek || self.reveal.is_some() || width <= 0. {
            return;
        }
        let offset = self.scroll.offset();
        let y = -f32::from(offset.y);
        let page = layout.at(y, width);
        let top = layout.top(page, width);
        let position = Position {
            page,
            y: (y - top) / (layout.top(page + 1, width) - top),
            x: -f32::from(offset.x) / width,
            zoom: self.zoom,
        };
        if self.position != position || self.position_dirty && self.save_position.is_none() {
            self.page = page;
            self.position = position;
            self.position_dirty = true;
            self.save_position = Some(cx.spawn(async move |reader, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
                let _ = reader.update(cx, |reader, cx| {
                    reader.persist_position();
                    cx.notify();
                });
            }));
            cx.notify();
        }
    }

    pub(super) fn update_pages(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        if width <= 0. {
            return;
        }
        let raster = (width * window.scale_factor()).round().clamp(1., 4096.) as i32;
        let viewport = self.scroll.bounds().size;
        if self.seek || self.position.zoom != self.zoom {
            let top = layout.top(self.page, width);
            let height = layout.top(self.page + 1, width) - top;
            self.scroll.set_offset(point(
                px(-self.position.x * width),
                px(-top - height * self.position.y),
            ));
            self.seek = false;
            self.position_dirty = true;
            self.save_position = None;
        }
        let y = -f32::from(self.scroll.offset().y);
        let mut visible = layout.at(y, width)..layout.at(y + f32::from(viewport.height), width) + 1;
        if self.reveal.is_some() {
            visible.start = visible.start.min(self.page);
            visible.end = visible.end.max(self.page + 1);
        }
        self.wanted = visible;
        self.pages.retain(|index, result| {
            let keep = self.wanted.contains(index) && self.render_width == raster;
            if !keep && let Ok(page) = result {
                cx.drop_image(page.image.clone(), Some(window));
            }
            keep
        });
        self.in_flight
            .retain(|index| self.wanted.contains(index) && self.render_width == raster);
        self.render_width = raster;
        for index in self.wanted.clone() {
            if !self.pages.contains_key(&index) && self.in_flight.insert(index) {
                let _ = self.requests.send(Command::Render(Request {
                    page: index,
                    width: raster,
                }));
            }
        }
        if let Some(result) = self.pages.get(&self.page)
            && let Some(range) = self.reveal.take()
            && let Ok(page) = result
        {
            let page_size = size(px(width), px(width / page.aspect));
            if let Some(region) = page
                .text
                .boxes(range, Bounds::new(Point::default(), page_size))
                .next()
            {
                let x = (viewport.width * 0.5 - region.center().x).min(px(0.));
                let y =
                    viewport.height * 0.5 - region.center().y - px(layout.top(self.page, width));
                self.scroll.set_offset(point(x, y.min(px(0.))));
            }
        }
    }
}
