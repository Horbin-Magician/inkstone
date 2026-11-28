//! Read-only, on-demand PDF page rendering.
use super::*;
use pdfium_render::prelude::{PdfDocument, PdfRenderConfig, Pdfium};
use std::{collections::VecDeque, path::Path, sync::OnceLock};

pub(super) fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Request {
    page: i32,
    width: i32,
}

struct Page {
    image: Arc<RenderImage>,
    aspect: f32,
    count: i32,
}

fn render_page(doc: &PdfDocument<'_>, request: Request) -> anyhow::Result<Page> {
    let page = doc.pages().get(request.page)?;
    let aspect = page.width().value / page.height().value;
    anyhow::ensure!(aspect.is_finite() && aspect > 0., "无效的页面尺寸");
    let config = PdfRenderConfig::new()
        .set_target_width(request.width)
        .set_maximum_height(4096)
        .set_maximum_width(4096);
    let bitmap = page.render_with_config(&config)?;
    let mut bytes = bitmap.as_rgba_bytes();
    // GPUI uploads BGRA frames to the atlas.
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(bitmap.width() as u32, bitmap.height() as u32, bytes)
        .ok_or_else(|| anyhow::anyhow!("无效的页面位图"))?;
    Ok(Page {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        aspect,
        count: doc.pages().len(),
    })
}

type Response = (Request, Result<Arc<Page>, String>);

fn worker(
    path: PathBuf,
    requests: std::sync::mpsc::Receiver<Request>,
    results: async_channel::Sender<Response>,
) {
    static ENGINE: OnceLock<Result<Pdfium, String>> = OnceLock::new();
    let engine = ENGINE.get_or_init(|| {
        let library = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name(Pdfium::pdfium_platform_library_name());
        Pdfium::bind_to_library(library)
            .map(Pdfium::new)
            .map_err(|e| format!("无法加载 PDF 阅读引擎：{e}"))
    });
    let document = engine.as_ref().map_err(Clone::clone).and_then(|pdf| {
        pdf.load_pdf_from_file(&path, None)
            .map_err(|e| format!("无法打开 PDF：{e}"))
    });
    let mut cache: VecDeque<(Request, Arc<Page>)> = VecDeque::new();
    while let Ok(first) = requests.recv() {
        let request = requests.try_iter().last().unwrap_or(first);
        let result = document.as_ref().map_err(Clone::clone).and_then(|doc| {
            if let Some(pos) = cache.iter().position(|(key, _)| *key == request) {
                let (_, page) = cache.remove(pos).unwrap();
                cache.push_front((request, page.clone()));
                return Ok(page);
            }
            let page =
                Arc::new(render_page(doc, request).map_err(|e| format!("无法显示此页：{e}"))?);
            cache.truncate(1);
            cache.push_front((request, page.clone()));
            Ok(page)
        });
        if results.send_blocking((request, result)).is_err() {
            break;
        }
    }
}

pub(super) struct Reader {
    pub(super) focus: FocusHandle,
    jump: Entity<InputState>,
    requests: std::sync::mpsc::Sender<Request>,
    pending: Option<Request>,
    page: i32,
    count: i32,
    zoom: f32,
    width: f32,
    visible: Option<Arc<Page>>,
    scroll: ScrollHandle,
    error: Option<String>,
}

impl Reader {
    pub(super) fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (requests, rx) = std::sync::mpsc::channel();
        let (tx, results) = async_channel::bounded(1);
        std::thread::spawn(move || worker(path, rx, tx));
        cx.spawn(async move |this, cx| {
            while let Ok((request, result)) = results.recv().await {
                if this
                    .update(cx, |reader: &mut Self, cx| {
                        if reader.pending != Some(request) {
                            return;
                        }
                        if let Some(old) = reader.visible.take() {
                            cx.drop_image(old.image.clone(), None);
                        }
                        match result {
                            Ok(page) => {
                                reader.count = page.count;
                                reader.visible = Some(page);
                                reader.error = None;
                            }
                            Err(message) => reader.error = Some(message),
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.on_release(|reader, cx| {
            if let Some(page) = reader.visible.take() {
                cx.drop_image(page.image.clone(), None);
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            jump: cx.new(|cx| InputState::new(window, cx).placeholder("页码")),
            requests,
            pending: None,
            page: 0,
            count: 0,
            zoom: 1.,
            width: 0.,
            visible: None,
            scroll: ScrollHandle::new(),
            error: None,
        }
    }
}

impl Render for Reader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logical = self.width * self.zoom;
        let request = Request {
            page: self.page,
            width: (logical * window.scale_factor()).round().clamp(1., 4096.) as i32,
        };
        if self.width > 0. && self.pending != Some(request) {
            if self.pending.is_some_and(|old| old.page != request.page) {
                self.scroll.set_offset(Point::default());
                if let Some(old) = self.visible.take() {
                    cx.drop_image(old.image.clone(), Some(window));
                }
            }
            self.pending = Some(request);
            let _ = self.requests.send(request);
        }
        let view = cx.entity().downgrade();
        let mut bar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .h(px(40.))
            .flex_shrink_0()
            .child(format!(
                "{} / {}",
                if self.count == 0 { 0 } else { self.page + 1 },
                self.count
            ))
            .child(Input::new(&self.jump).w(px(64.)));
        for (id, label) in [
            ("prev", "上一页"),
            ("next", "下一页"),
            ("jump", "跳转"),
            ("out", "缩小"),
            ("in", "放大"),
            ("fit", "适应宽度"),
        ] {
            bar = bar.child(
                div()
                    .id(id)
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match id {
                            "prev" => this.page = (this.page - 1).max(0),
                            "next" => this.page = (this.page + 1).min((this.count - 1).max(0)),
                            "jump" => {
                                if let Ok(n) = this.jump.read(cx).value().trim().parse::<i32>()
                                    && (1..=this.count).contains(&n)
                                {
                                    this.page = n - 1;
                                }
                            }
                            "out" => this.zoom = (this.zoom / 1.25).max(0.25),
                            "in" => this.zoom = (this.zoom * 1.25).min(4.),
                            _ => this.zoom = 1.,
                        }
                        cx.notify();
                    })),
            );
        }
        let content = if let Some(error) = &self.error {
            div().child(error.clone())
        } else if let Some(page) = &self.visible {
            div().child(
                img(page.image.clone())
                    .w(px(logical))
                    .h(px(logical / page.aspect)),
            )
        } else {
            div().child("正在加载 PDF…")
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                let width = f32::from(bounds.size.width).floor();
                                let _ = view.update(cx, |reader, cx| {
                                    if reader.width != width {
                                        reader.width = width;
                                        cx.notify();
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .id("pdf-scroll")
                            .track_scroll(&self.scroll)
                            .size_full()
                            .overflow_scroll()
                            .child(content),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    #[ignore = "requires packaged PDFium beside the test executable"]
    fn renders_two_pages_at_requested_resolution() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-pages.pdf");
        let (send, requests) = std::sync::mpsc::channel();
        let (results, recv) = async_channel::bounded(1);
        let thread = std::thread::spawn(move || worker(path, requests, results));
        let get = |page, width| {
            send.send(Request { page, width }).unwrap();
            recv.recv_blocking().unwrap().1.unwrap()
        };
        let first = get(0, 300);
        assert_eq!(first.count, 2);
        assert_eq!(first.aspect, 0.75);
        assert_eq!(&first.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert_eq!(i32::from(get(1, 600).image.size(0).width), 600);
        assert!(Arc::ptr_eq(&first, &get(0, 300)));
        drop(send);
        thread.join().unwrap();
    }
}
