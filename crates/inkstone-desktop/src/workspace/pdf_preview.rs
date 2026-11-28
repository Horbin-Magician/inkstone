//! Read-only, on-demand PDF page rendering.
mod search;
mod text;
use super::*;
use pdfium_render::prelude::{PdfDocument, PdfRenderConfig, Pdfium};
use search::{Hit, Scan};
use std::{cell::Cell, collections::VecDeque, ops::Range, path::Path, rc::Rc, sync::OnceLock};
use text::TextLayer;

actions!(pdf_reader, [FindText]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            &crate::shortcuts::command_default("ctrl-f"),
            FindText,
            Some("PdfReader"),
        ),
        KeyBinding::new(
            &crate::shortcuts::command_default("ctrl-c"),
            gpui_component::input::Copy,
            Some("PdfReader"),
        ),
    ]);
}

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
    text: TextLayer,
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
        text: TextLayer::read(&page, &config, bitmap.width(), bitmap.height()),
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        aspect,
        count: doc.pages().len(),
    })
}

enum Command {
    Render(Request),
    Search(u64, String),
}
enum Response {
    Page(Request, Result<Arc<Page>, String>),
    Matches(u64, Result<Vec<Hit>, String>),
}

fn worker(
    path: PathBuf,
    requests: std::sync::mpsc::Receiver<Command>,
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
    let mut scan: Option<Scan> = None;
    loop {
        use std::sync::mpsc::TryRecvError;
        let first = if scan.is_some() {
            match requests.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => None,
            }
        } else {
            let Ok(command) = requests.recv() else { break };
            Some(command)
        };
        let mut render = None;
        for command in first.into_iter().chain(requests.try_iter()) {
            match command {
                Command::Render(request) => render = Some(request),
                Command::Search(id, query) => {
                    scan = None;
                    if !query.is_empty() {
                        match Scan::new(id, &query) {
                            Ok(new) => scan = Some(new),
                            Err(e) => {
                                if results
                                    .send_blocking(Response::Matches(id, Err(e.to_string())))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
        let Some(request) = render else {
            if let Some(mut current) = scan.take() {
                let done = document
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|doc| current.advance(doc).map_err(|e| e.to_string()));
                if matches!(done, Ok(false)) {
                    scan = Some(current);
                } else if results
                    .send_blocking(Response::Matches(
                        current.generation,
                        done.map(|_| current.hits),
                    ))
                    .is_err()
                {
                    break;
                }
            }
            continue;
        };
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
        if results
            .send_blocking(Response::Page(request, result))
            .is_err()
        {
            break;
        }
    }
}

pub(super) struct Reader {
    pub(super) focus: FocusHandle,
    jump: Entity<InputState>,
    query: Entity<InputState>,
    generation: u64,
    hits: Vec<Hit>,
    current: usize,
    reveal: bool,
    search_status: String,
    _query_events: Subscription,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    selection: Option<Range<usize>>,
    anchor: Option<Range<usize>>,
    requests: std::sync::mpsc::Sender<Command>,
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
        Self::with_worker(requests, results, window, cx)
    }

    fn with_worker(
        requests: std::sync::mpsc::Sender<Command>,
        results: async_channel::Receiver<Response>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while let Ok(response) = results.recv().await {
                if this
                    .update(cx, |reader: &mut Self, cx| {
                        let (request, result) = match response {
                            Response::Page(request, result) => (request, result),
                            Response::Matches(id, result) => {
                                if id == reader.generation {
                                    match result {
                                        Ok(hits) => {
                                            reader.hits = hits;
                                            reader.current = reader
                                                .hits
                                                .partition_point(|h| h.page < reader.page);
                                            if reader.current == reader.hits.len() {
                                                reader.current = 0;
                                            }
                                            reader.search_status = "无匹配".into();
                                            reader.goto_match();
                                        }
                                        Err(e) => reader.search_status = format!("搜索失败：{e}"),
                                    }
                                    cx.notify();
                                }
                                return;
                            }
                        };
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
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("在 PDF 中查找"));
        let events = cx.subscribe(
            &query,
            |reader: &mut Self, input, event: &InputEvent, cx| {
                match event {
                    InputEvent::Change => {
                        reader.generation += 1;
                        reader.hits.clear();
                        reader.reveal = false;
                        let query = input.read(cx).value().to_string();
                        reader.search_status = if query.is_empty() {
                            String::new()
                        } else {
                            "搜索中…".into()
                        };
                        let _ = reader
                            .requests
                            .send(Command::Search(reader.generation, query));
                    }
                    _ => return,
                }
                cx.notify();
            },
        );
        Self {
            query,
            generation: 0,
            hits: Vec::new(),
            current: 0,
            reveal: false,
            search_status: String::new(),
            _query_events: events,
            focus: cx.focus_handle(),
            jump: cx.new(|cx| InputState::new(window, cx).placeholder("页码")),
            requests,
            bounds: Default::default(),
            selection: None,
            anchor: None,
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
    fn goto_match(&mut self) {
        if let Some(hit) = self.hits.get(self.current) {
            self.page = hit.page;
            self.reveal = true;
        }
    }

    fn step_match(&mut self, step: isize) {
        if !self.hits.is_empty() {
            self.current =
                (self.current as isize + step).rem_euclid(self.hits.len() as isize) as usize;
            self.goto_match();
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
                self.selection = None;
                self.anchor = None;
                self.scroll.set_offset(Point::default());
                if let Some(old) = self.visible.take() {
                    cx.drop_image(old.image.clone(), Some(window));
                }
            }
            self.pending = Some(request);
            let _ = self.requests.send(Command::Render(request));
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
            let bounds = self.bounds.clone();
            let selection = self.selection.clone();
            let start = self.hits.partition_point(|h| h.page < self.page);
            let marks: Vec<_> = self.hits[start..]
                .iter()
                .take_while(|h| h.page == self.page)
                .enumerate()
                .map(|(i, h)| (h.bytes.clone(), start + i == self.current))
                .collect();
            if self.reveal
                && let Some(hit) = self.hits.get(self.current)
                && hit.page == self.page
            {
                let size = size(px(logical), px(logical / page.aspect));
                let viewport = self.scroll.bounds().size;
                if let Some(region) = page
                    .text
                    .boxes(hit.bytes.clone(), Bounds::new(Point::default(), size))
                    .next()
                {
                    let offset = point(
                        (viewport.width * 0.5 - region.center().x)
                            .clamp((viewport.width - size.width).min(px(0.)), px(0.)),
                        (viewport.height * 0.5 - region.center().y)
                            .clamp((viewport.height - size.height).min(px(0.)), px(0.)),
                    );
                    self.scroll.set_offset(offset);
                }
                self.reveal = false;
            }
            let shown = page.clone();
            div()
                .relative()
                .w(px(logical))
                .h(px(logical / page.aspect))
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.anchor = this
                            .visible
                            .as_ref()
                            .and_then(|p| p.text.hit(event.position, this.bounds.get(), false));
                        this.selection = this.anchor.clone();
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.anchor = None;
                        return;
                    }
                    if let (Some(anchor), Some(hit)) = (
                        &this.anchor,
                        this.visible
                            .as_ref()
                            .and_then(|p| p.text.hit(event.position, this.bounds.get(), true)),
                    ) {
                        this.selection = Some(anchor.start.min(hit.start)..anchor.end.max(hit.end));
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.anchor = None),
                )
                .child(img(page.image.clone()).size_full())
                .child(
                    canvas(
                        move |rect, _, _| bounds.set(rect),
                        move |rect, _, window, _| {
                            for (range, active) in &marks {
                                let color = if *active { 0xff84149c } else { 0xffdb3075 };
                                for region in shown.text.boxes(range.clone(), rect) {
                                    window.paint_quad(fill(region, rgba(color)));
                                }
                            }
                            if let Some(range) = &selection {
                                for region in shown.text.boxes(range.clone(), rect) {
                                    window.paint_quad(fill(region, rgba(0x4097ff70)));
                                }
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
        } else {
            div().child("正在加载 PDF…")
        };
        div()
            .key_context("PdfReader")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FindText, window, cx| {
                this.query.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.select_all(window, cx);
                });
            }))
            .on_action(
                cx.listener(|this, _: &gpui_component::input::Copy, window, cx| {
                    if this.focus.is_focused(window)
                        && let Some(text) = this
                            .visible
                            .as_ref()
                            .and_then(|p| p.text.text.get(this.selection.clone()?))
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .flex_shrink_0()
                    .child(
                        div()
                            .on_action(cx.listener(
                                |this, action: &gpui_component::input::Enter, _, cx| {
                                    this.step_match(if action.shift { -1 } else { 1 });
                                    cx.notify();
                                },
                            ))
                            .child(Input::new(&self.query).w(px(200.))),
                    )
                    .child(if self.hits.is_empty() {
                        self.search_status.clone()
                    } else {
                        format!("{} / {}", self.current + 1, self.hits.len())
                    })
                    .children(
                        [("prev-match", "上一处", -1), ("next-match", "下一处", 1)]
                            .into_iter()
                            .map(|(id, label, step)| {
                                div()
                                    .id(id)
                                    .cursor_pointer()
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.step_match(step);
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
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

    #[gpui::test]
    #[ignore = "requires packaged PDFium beside the test executable"]
    fn select_copy_and_navigate_matches(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(init);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-pages.pdf");
        let (requests, rx) = std::sync::mpsc::channel();
        let (tx, worker_results) = async_channel::bounded(1);
        std::thread::spawn(move || worker(path, rx, tx));
        // Deliver native worker responses on the deterministic GPUI test scheduler.
        let (dispatch, results) = async_channel::unbounded();
        let handle = cx.add_window(|window, cx| Reader::with_worker(requests, results, window, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(600.)));
        let wait = |visual: &mut VisualTestContext, predicate: fn(&Reader) -> bool| {
            for _ in 0..300 {
                while let Ok(response) = worker_results.try_recv() {
                    dispatch.try_send(response).unwrap();
                }
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                if handle.update(visual, |r, _, _| predicate(r)).unwrap() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("reader did not reach expected state");
        };
        wait(&mut visual, |r| r.visible.is_some());
        let (first, last) = handle
            .update(&mut visual, |r, _, _| {
                let layer = &r.visible.as_ref().unwrap().text;
                (
                    layer.boxes(0..1, r.bounds.get()).next().unwrap().center(),
                    layer.boxes(7..8, r.bounds.get()).next().unwrap().center(),
                )
            })
            .unwrap();
        visual.simulate_mouse_down(first, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(last, Some(MouseButton::Left), Modifiers::default());
        visual.simulate_mouse_up(last, MouseButton::Left, Modifiers::default());
        visual.simulate_keystrokes(&crate::shortcuts::command_default("ctrl-c"));
        visual.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("Inkstone")
            )
        });
        visual.simulate_keystrokes(&crate::shortcuts::command_default("ctrl-f"));
        handle
            .update(&mut visual, |r, w, cx| {
                assert!(r.query.read(cx).focus_handle(cx).is_focused(w));
            })
            .unwrap();
        visual.simulate_input("inkstone");
        wait(&mut visual, |r| r.hits.len() == 2);
        visual.simulate_keystrokes("enter");
        wait(&mut visual, |r| {
            r.page == 1
                && r.visible
                    .as_ref()
                    .is_some_and(|p| p.text.text == "INKSTONE notes")
        });
        handle
            .update(&mut visual, |r, _, _| {
                let p = r.visible.as_ref().unwrap();
                let region = p
                    .text
                    .boxes(r.hits[r.current].bytes.clone(), r.bounds.get())
                    .next()
                    .unwrap();
                assert!(r.scroll.bounds().contains(&region.center()));
                assert!(r.selection.is_none());
            })
            .unwrap();
    }

    #[test]
    #[ignore = "requires packaged PDFium beside the test executable"]
    fn renders_two_pages_at_requested_resolution() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-pages.pdf");
        let (send, requests) = std::sync::mpsc::channel();
        let (results, recv) = async_channel::bounded(1);
        let thread = std::thread::spawn(move || worker(path, requests, results));
        let get = |page, width| {
            send.send(Command::Render(Request { page, width })).unwrap();
            let Response::Page(_, result) = recv.recv_blocking().unwrap() else {
                panic!("expected a page")
            };
            result.unwrap()
        };
        let first = get(0, 300);
        assert_eq!(first.count, 2);
        assert_eq!(first.text.text, "Inkstone reader");
        let screen = Bounds::new(point(px(-17.), px(44.)), size(px(600.), px(800.)));
        let glyph = first.text.boxes(0..1, screen).next().unwrap();
        assert_eq!(first.text.hit(glyph.center(), screen, false), Some(0..1));
        assert!(glyph.origin.y > px(180.) && glyph.origin.y < px(260.));
        assert_eq!(first.aspect, 0.75);
        assert_eq!(&first.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert_eq!(i32::from(get(1, 600).image.size(0).width), 600);
        assert!(Arc::ptr_eq(&first, &get(0, 300)));
        send.send(Command::Search(7, "inkstone".into())).unwrap();
        let Response::Matches(7, result) = recv.recv_blocking().unwrap() else {
            panic!("expected matches")
        };
        assert_eq!(
            result.unwrap(),
            vec![
                Hit {
                    page: 0,
                    bytes: 0..8
                },
                Hit {
                    page: 1,
                    bytes: 0..8
                }
            ]
        );
        drop(send);
        thread.join().unwrap();
    }
}
