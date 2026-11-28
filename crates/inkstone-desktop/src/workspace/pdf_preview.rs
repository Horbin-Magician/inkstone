//! On-demand PDF page rendering, text search and annotations.
mod annotations;
mod excerpt;
pub(super) use excerpt::{Excerpt, SourceAnchor};
mod layout;
use layout::{Layout, Position};
use std::collections::BTreeMap;
mod markup_ui;
use annotations::{Color, Mark, Store, Style};
mod search;
mod sidebar;
use sidebar::Sidebar;
mod text;
use super::*;
use pdfium_render::prelude::{PdfDocument, PdfRenderConfig, Pdfium};
use search::{Hit, Scan};
use std::{cell::Cell, collections::VecDeque, ops::Range, path::Path, rc::Rc, sync::OnceLock};
use text::TextLayer;

actions!(pdf_reader, [FindText, CloseFind, StopMarking]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", StopMarking, Some("PdfReader")),
        KeyBinding::new("escape", CloseFind, Some("PdfFind")),
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
}

fn render_page(doc: &PdfDocument<'_>, request: Request) -> anyhow::Result<Page> {
    let page = doc.pages().get(request.page)?;
    let aspect = page.width().value / page.height().value;
    anyhow::ensure!(aspect.is_finite() && aspect > 0., "无效的页面尺寸");
    let config = PdfRenderConfig::new()
        .set_target_width(request.width)
        .set_maximum_height(4096)
        .set_maximum_width(4096);
    let image = bitmap(&page, &config)?;
    Ok(Page {
        text: TextLayer::read(
            &page,
            &config,
            i32::from(image.size(0).width),
            i32::from(image.size(0).height),
        ),
        image,
        aspect,
    })
}

fn bitmap(
    page: &pdfium_render::prelude::PdfPage<'_>,
    config: &PdfRenderConfig,
) -> anyhow::Result<Arc<RenderImage>> {
    let bitmap = page.render_with_config(config)?;
    let mut bytes = bitmap.as_rgba_bytes();
    // GPUI uploads BGRA frames to the atlas.
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(bitmap.width() as u32, bitmap.height() as u32, bytes)
        .ok_or_else(|| anyhow::anyhow!("无效的页面位图"))?;
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

enum Command {
    Layout,
    Render(Request),
    Thumbnail(i32),
    Search(u64, String),
}
enum Response {
    Layout(Result<Layout, String>),
    Page(Request, Result<Arc<Page>, String>),
    Matches(u64, Result<Vec<Hit>, String>),
    Thumbnail(i32, Result<Arc<RenderImage>, String>),
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
        let mut render = Vec::new();
        for command in first.into_iter().chain(requests.try_iter()) {
            match command {
                Command::Layout => {
                    let layout = document
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|doc| Layout::read(doc).map_err(|e| e.to_string()));
                    if results.send_blocking(Response::Layout(layout)).is_err() {
                        return;
                    }
                }
                Command::Thumbnail(index) => {
                    let image = document.as_ref().map_err(Clone::clone).and_then(|doc| {
                        let page = doc.pages().get(index).map_err(|e| e.to_string())?;
                        bitmap(
                            &page,
                            &PdfRenderConfig::new()
                                .set_target_width(170)
                                .set_maximum_height(200),
                        )
                        .map_err(|e| e.to_string())
                    });
                    if results
                        .send_blocking(Response::Thumbnail(index, image))
                        .is_err()
                    {
                        return;
                    }
                }
                Command::Render(request) => render.push(request),
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
        if render.is_empty() {
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
        for request in render {
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
                return;
            }
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
    reveal: Option<Range<usize>>,
    store: Store,
    annotations: Vec<Mark>,
    editing: Option<usize>,
    comment: Entity<InputState>,
    color: Color,
    continuous: Option<Style>,
    popup: Option<Point<f32>>,
    press: Point<Pixels>,
    sidebar: Sidebar,
    find_open: bool,
    mark_error: Option<String>,
    search_status: String,
    _query_events: Subscription,
    layout: Option<Layout>,
    pages: BTreeMap<i32, Result<Arc<Page>, String>>,
    in_flight: std::collections::BTreeSet<i32>,
    wanted: Range<i32>,
    render_width: i32,
    position: Position,
    position_dirty: bool,
    save_position: Option<Task<()>>,
    seek: bool,
    selected_page: i32,
    source_anchor: Option<SourceAnchor>,
    pub(super) note_target: Option<(usize, PathBuf)>,
    selection: Option<Range<usize>>,
    anchor: Option<Range<usize>>,
    requests: std::sync::mpsc::Sender<Command>,

    page: i32,
    count: i32,
    zoom: f32,
    width: f32,

    scroll: ScrollHandle,
    error: Option<String>,
}

impl Reader {
    pub(super) fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (requests, rx) = std::sync::mpsc::channel();
        let (tx, results) = async_channel::bounded(1);
        let store = Store::new(&path);
        std::thread::spawn(move || worker(path, rx, tx));
        // Native PDFium threads may not wake GPUI's single-threaded test scheduler.
        // Relay responses during simulated time, as in the reader interaction tests.
        #[cfg(test)]
        let results = {
            let (dispatch, local) = async_channel::unbounded();
            cx.spawn(async move |reader, cx| {
                while reader.upgrade().is_some() && !results.is_closed() {
                    while let Ok(response) = results.try_recv() {
                        if dispatch.try_send(response).is_err() {
                            return;
                        }
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(10))
                        .await;
                }
            })
            .detach();
            local
        };
        Self::with_worker(store, requests, results, window, cx)
    }

    fn with_worker(
        store: Store,
        requests: std::sync::mpsc::Sender<Command>,
        results: async_channel::Receiver<Response>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while let Ok(response) = results.recv().await {
                if this
                    .update(cx, |reader: &mut Self, cx| {
                        match response {
                            Response::Layout(result) => match result {
                                Ok(layout) => {
                                    reader.count = layout.count();
                                    reader.page =
                                        reader.page.clamp(0, reader.count.saturating_sub(1));
                                    reader.layout = Some(layout);
                                }
                                Err(message) => reader.error = Some(message),
                            },
                            Response::Page(request, result) => {
                                if reader.render_width == request.width
                                    && reader.wanted.contains(&request.page)
                                {
                                    reader.in_flight.remove(&request.page);
                                    reader.pages.insert(request.page, result);
                                }
                            }
                            Response::Thumbnail(index, result) => {
                                if let Some(slot) = reader.sidebar.thumbnails.get_mut(&index) {
                                    *slot = Some(result);
                                }
                            }
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
            reader.persist_position();
            reader.sidebar.clear(cx);
            for page in reader.pages.values().flatten() {
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
                        reader.reveal = None;
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
        let (annotations, mark_error) = match store.read() {
            Ok(marks) => (marks, None),
            Err(e) => (Vec::new(), Some(format!("读取标记失败：{e}"))),
        };
        let position = store.position();
        let _ = requests.send(Command::Layout);
        Self {
            layout: None,
            pages: Default::default(),
            in_flight: Default::default(),
            wanted: 0..0,
            render_width: 0,
            position,
            position_dirty: false,
            save_position: None,
            seek: true,
            selected_page: -1,
            source_anchor: None,
            note_target: None,
            store,
            annotations,
            mark_error,
            continuous: None,
            popup: None,
            press: Default::default(),
            color: Color::Yellow,
            editing: None,
            comment: cx.new(|cx| InputState::new(window, cx).placeholder("批注（可选）")),
            sidebar: Sidebar::default(),
            find_open: false,
            query,
            generation: 0,
            hits: Vec::new(),
            current: 0,
            reveal: None,
            search_status: String::new(),
            _query_events: events,
            focus: cx.focus_handle(),
            jump: cx.new(|cx| InputState::new(window, cx).placeholder("页码")),
            requests,

            selection: None,
            anchor: None,

            page: position.page,
            count: 0,
            zoom: position.zoom,
            width: 0.,

            scroll: ScrollHandle::new(),
            error: None,
        }
    }
    fn goto_match(&mut self) {
        if let Some(hit) = self.hits.get(self.current).cloned() {
            self.navigate(hit.page);
            self.reveal = Some(hit.bytes.clone());
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

impl Reader {
    fn page_view(&self, index: i32, page: &Arc<Page>, logical: f32, cx: &Context<Self>) -> Div {
        let bounds: Rc<Cell<Bounds<Pixels>>> = Default::default();
        let hit_bounds = bounds.clone();
        let drag_bounds = bounds.clone();
        let release_bounds = bounds.clone();
        let selection = (index == self.selected_page)
            .then(|| self.selection.clone())
            .flatten();
        let start = self.hits.partition_point(|h| h.page < index);
        let marks: Vec<_> = self.hits[start..]
            .iter()
            .take_while(|h| h.page == index)
            .enumerate()
            .map(|(i, h)| (h.bytes.clone(), start + i == self.current))
            .collect();
        let annotations: Vec<_> = self
            .annotations
            .iter()
            .filter(|m| {
                m.page == index && page.text.text.get(m.bytes.clone()) == Some(m.quote.as_str())
            })
            .map(|m| (m.bytes.clone(), m.style, m.color))
            .collect();
        let shown = page.clone();
        div()
            .debug_selector(move || format!("pdf-page-{index}").into())
            .relative()
            .w(px(logical))
            .h(px(logical / page.aspect))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.reset_editor(window, cx);
                    this.press = event.position;
                    this.selected_page = index;
                    this.anchor = this
                        .active_page()
                        .and_then(|p| p.text.hit(event.position, hit_bounds.get(), false));
                    this.selection = this.anchor.clone();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if this.selected_page != index {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    this.anchor = None;
                    return;
                }
                if let (Some(anchor), Some(hit)) = (
                    &this.anchor,
                    this.active_page()
                        .and_then(|p| p.text.hit(event.position, drag_bounds.get(), true)),
                ) {
                    this.selection = Some(anchor.start.min(hit.start)..anchor.end.max(hit.end));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                    if index != this.selected_page || this.anchor.take().is_none() {
                        return;
                    }
                    let click = (this.press.x - event.position.x)
                        .abs()
                        .max((this.press.y - event.position.y).abs())
                        < px(3.);
                    if let Some(i) = this.annotations.iter().rposition(|m| {
                        m.page == this.selected_page
                            && this.selection.as_ref().is_some_and(|r| {
                                r == &m.bytes || (click && m.bytes.contains(&r.start))
                            })
                            && this
                                .active_page()
                                .and_then(|p| p.text.text.get(m.bytes.clone()))
                                == Some(m.quote.as_str())
                    }) {
                        this.selection = Some(this.annotations[i].bytes.clone());
                        this.edit_mark(i, window, cx);
                    } else if !click
                        && this
                            .selection
                            .clone()
                            .and_then(|r| this.active_page()?.text.text.get(r))
                            .is_some_and(|s| !s.trim().is_empty())
                    {
                        if let Some(style) = this.continuous {
                            this.save_mark(Some(style), window, cx);
                        } else {
                            let rect = release_bounds.get();
                            this.popup = Some(point(
                                (event.position.x - rect.left()) / rect.size.width,
                                (event.position.y - rect.top()) / rect.size.height,
                            ));
                            cx.notify();
                        }
                    }
                }),
            )
            .child(img(page.image.clone()).size_full())
            .child(
                canvas(
                    move |rect, _, _| bounds.set(rect),
                    move |rect, _, window, _| {
                        for (range, style, color) in &annotations {
                            for mut region in shown.text.boxes(range.clone(), rect) {
                                let alpha = match style {
                                    Style::Highlight => 0x66,
                                    Style::Underline => {
                                        let height = region.size.height.min(px(2.));
                                        region.origin.y += region.size.height - height;
                                        region.size.height = height;
                                        0xff
                                    }
                                };
                                window.paint_quad(fill(region, rgba(color.rgb() << 8 | alpha)));
                            }
                        }
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
            .when(index == self.selected_page, |el| {
                el.when_some(self.popup, |el, at| {
                    el.child(self.selection_tools(at, logical, page.aspect, cx))
                })
            })
    }
}
impl Render for Reader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let logical = self.width * self.zoom;
        self.update_pages(logical, window, cx);
        let view = cx.entity().downgrade();
        let mut bar = div()
            .child(
                div()
                    .id("sidebar-toggle")
                    .debug_selector(|| "sidebar-toggle".into())
                    .cursor_pointer()
                    .child(if self.sidebar.open {
                        "收起侧栏"
                    } else {
                        "侧栏"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar.open = !this.sidebar.open;
                        cx.notify();
                    })),
            )
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
                            "prev" => this.navigate((this.page - 1).max(0)),
                            "next" => this.navigate((this.page + 1).min((this.count - 1).max(0))),
                            "jump" => {
                                if let Ok(n) = this.jump.read(cx).value().trim().parse::<i32>()
                                    && (1..=this.count).contains(&n)
                                {
                                    this.navigate(n - 1);
                                }
                            }
                            "out" => this.zoom = (this.zoom / 1.25).max(0.25),
                            "in" => this.zoom = (this.zoom * 1.25).min(4.),
                            _ => this.zoom = 1.,
                        }
                        if matches!(id, "prev" | "next" | "jump") {
                            this.reveal = None;
                        }
                        cx.notify();
                    })),
            );
        }
        bar = bar.child(
            div()
                .id("show-find")
                .cursor_pointer()
                .child("查找")
                .on_click(cx.listener(|this, _, w, cx| this.open_find(w, cx))),
        );
        let mut content = div().relative().w(px(logical)).h(px(self
            .layout
            .as_ref()
            .map_or(0., |l| l.top(self.count, logical))));
        if self.layout.is_none() {
            content = div().child(self.error.clone().unwrap_or_else(|| "正在加载 PDF…".into()));
        }
        for index in self.wanted.clone() {
            let top = self.layout.as_ref().unwrap().top(index, logical);
            let mut slot = div().absolute().top(px(top)).left_0();
            match self.pages.get(&index) {
                Some(Ok(page)) => slot = slot.child(self.page_view(index, page, logical, cx)),
                Some(Err(message)) => slot = slot.child(message.clone()),
                None => slot = slot.child("正在加载 PDF…"),
            }
            content = content.child(slot);
        }
        div()
            .key_context("PdfReader")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &StopMarking, window, cx| {
                if this.find_open {
                    this.close_find(window, cx);
                } else {
                    this.continuous = None;
                    this.popup = None;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindText, window, cx| {
                this.open_find(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &gpui_component::input::Copy, window, cx| {
                    if this.focus.is_focused(window)
                        && let Some(text) = this
                            .active_page()
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
            .child(self.markup_bar(cx))
            .children(self.mark_error.clone())
            .when(self.find_open, |el| {
                el.child(
                    div()
                        .key_context("PdfFind")
                        .debug_selector(|| "pdf-find".into())
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
                                    div().id(id).cursor_pointer().child(label).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.step_match(step);
                                            cx.notify();
                                        }),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id("close-find")
                                .cursor_pointer()
                                .child("关闭查找")
                                .on_click(cx.listener(|this, _, w, cx| this.close_find(w, cx))),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.sidebar.open, |el| el.child(self.sidebar_view(cx)))
                    .child(
                        div()
                            .min_w_0()
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
                                                reader.seek = true;
                                                cx.notify();
                                            } else {
                                                reader.observe_position(cx);
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
    fn annotate_selection_and_navigate_search_results(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(init);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-pages.pdf");
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("test.pdf");
        std::fs::copy(path, &pdf).unwrap();
        let store = Store::new(&pdf);
        let (requests, rx) = std::sync::mpsc::channel();
        let (tx, worker_results) = async_channel::bounded(1);
        std::thread::spawn(move || worker(pdf, rx, tx));
        // Deliver native worker responses on the deterministic GPUI test scheduler.
        let (dispatch, results) = async_channel::unbounded();
        let handle =
            cx.add_window(|window, cx| Reader::with_worker(store, requests, results, window, cx));
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
            handle.update(visual, |r,_,_| panic!("reader timeout: page {}, width {}, range {:?}, offset {:?}, search hits {}, error {:?}",r.page,r.width,r.wanted,r.scroll.offset(),r.hits.len(),r.error)).unwrap();
        };
        wait(&mut visual, |r| r.pages.contains_key(&r.page));
        let viewport = handle
            .update(&mut visual, |r, _, _| r.scroll.bounds())
            .unwrap();
        visual.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-850.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        wait(&mut visual, |r| r.wanted == (0..2) && r.pages.len() == 2);
        let first_bounds = visual.debug_bounds("pdf-page-0").unwrap();
        let next_bounds = visual.debug_bounds("pdf-page-1").unwrap();
        assert!(viewport.top() < first_bounds.bottom() && next_bounds.top() < viewport.bottom());
        visual.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(850.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        wait(&mut visual, |r| r.scroll.offset().y == px(0.));
        let bounds = visual.debug_bounds("pdf-page-0").unwrap();
        let (first, last) = handle
            .update(&mut visual, |r, _, _| {
                let layer = &r.pages[&r.page].as_ref().unwrap().text;
                (
                    layer.boxes(0..1, bounds).next().unwrap().center(),
                    layer.boxes(7..8, bounds).next().unwrap().center(),
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
        let click = |visual: &mut VisualTestContext, selector: &'static str| {
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let bounds = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(bounds.center(), Modifiers::default());
        };
        click(&mut visual, "green");
        click(&mut visual, "local-highlight");
        let saved = Store::new(&dir.path().join("test.pdf")).read().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].quote, "Inkstone");
        assert_eq!(saved[0].color, Color::Green);
        click(&mut visual, "lock-underline");
        let (start, end) = handle
            .update(&mut visual, |r, _, _| {
                let layer = &r.pages[&0].as_ref().unwrap().text;
                (
                    layer.boxes(9..10, bounds).next().unwrap().center(),
                    layer.boxes(14..15, bounds).next().unwrap().center(),
                )
            })
            .unwrap();
        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        handle
            .update(&mut visual, |r, _, _| {
                assert_eq!(r.annotations[1].quote, "reader");
                assert_eq!(r.annotations[1].style, Style::Underline);
                assert_eq!(r.continuous, Some(Style::Underline));
                assert!(r.popup.is_none());
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
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
            r.current == 1
                && r.reveal.is_none()
                && r.pages
                    .get(&1)
                    .and_then(|p| p.as_ref().ok())
                    .is_some_and(|p| p.text.text == "INKSTONE notes")
        });
        let bounds = visual.debug_bounds("pdf-page-1").unwrap();
        handle
            .update(&mut visual, |r, _, _| {
                let p = r.pages[&1].as_ref().unwrap();
                let region = p
                    .text
                    .boxes(r.hits[r.current].bytes.clone(), bounds)
                    .next()
                    .unwrap();
                assert!(r.scroll.bounds().contains(&region.center()));
                assert!(r.selection.is_none());
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        assert!(visual.debug_bounds("pdf-find").is_none());
        click(&mut visual, "sidebar-toggle");
        click(&mut visual, "pdf-thumbnails");
        wait(&mut visual, |r| {
            r.sidebar
                .thumbnails
                .get(&0)
                .is_some_and(|t| matches!(t, Some(Ok(_))))
        });
        click(&mut visual, "pdf-outline");
        click(&mut visual, "pdf-mark-list");
        click(&mut visual, "pdf-mark-0");
        wait(&mut visual, |r| {
            r.page == 0 && r.pages.contains_key(&r.page)
        });
        handle
            .update(&mut visual, |r, w, cx| {
                r.comment
                    .update(cx, |input, cx| input.set_value("重读", w, cx));
            })
            .unwrap();
        click(&mut visual, "underline");
        let saved = Store::new(&dir.path().join("test.pdf")).read().unwrap();
        assert_eq!(saved[0].style, Style::Underline);
        assert_eq!(saved[0].comment, "重读");
        handle
            .update(&mut visual, |r, _, cx| {
                r.zoom = 1.25;
                cx.notify();
            })
            .unwrap();
        wait(&mut visual, |r| {
            r.pages
                .get(&r.page)
                .and_then(|p| p.as_ref().ok())
                .is_some_and(|p| {
                    i32::from(p.image.size(0).width) == r.render_width
                        && p.text.text.get(r.annotations[0].bytes.clone()) == Some("Inkstone")
                })
        });
        handle
            .update(&mut visual, |r, _, _| r.persist_position())
            .unwrap();
        let expected = handle
            .update(&mut visual, |r, _, _| (r.position, r.zoom))
            .unwrap();
        let (requests, _rx) = std::sync::mpsc::channel();
        let (_tx, results) = async_channel::bounded(1);
        let reopened = cx.add_window(|w, cx| {
            Reader::with_worker(
                Store::new(&dir.path().join("test.pdf")),
                requests,
                results,
                w,
                cx,
            )
        });
        reopened
            .update(cx, |r, _, _| {
                assert_eq!(r.annotations, saved);
                assert!(r.position == expected.0);
                assert_eq!(r.zoom, expected.1);
            })
            .unwrap();
        click(&mut visual, "pdf-mark-0");
        click(&mut visual, "delete-mark");
        assert!(
            Store::new(&dir.path().join("test.pdf"))
                .read()
                .unwrap()
                .len()
                == 1
        );
        handle
            .update(&mut visual, |r, _, cx| {
                let href = format!(
                    "paper.pdf#page=2&inkstone=0-8-{}",
                    inkstone_agent::tools::hash("INKSTONE")
                );
                r.show_excerpt(SourceAnchor::parse(&href).unwrap());
                cx.notify();
            })
            .unwrap();
        wait(&mut visual, |r| {
            r.selected_page == 1 && r.selection == Some(0..8) && r.reveal.is_none()
        });
        let bounds = visual.debug_bounds("pdf-page-1").unwrap();
        handle
            .update(&mut visual, |r, _, _| {
                let glyph = r
                    .active_page()
                    .unwrap()
                    .text
                    .boxes(0..8, bounds)
                    .next()
                    .unwrap();
                assert!(r.scroll.bounds().contains(&glyph.center()));
            })
            .unwrap();
        // Save a nonzero offset on the second page and reopen at a different viewport width.
        handle
            .update(&mut visual, |r, _, cx| {
                r.navigate(1);
                r.zoom = 2.;
                cx.notify();
            })
            .unwrap();
        wait(&mut visual, |r| r.page == 1 && r.pages.contains_key(&1));
        visual.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.executor().advance_clock(Duration::from_millis(400));
        visual.run_until_parked();
        let pdf = dir.path().join("test.pdf");
        let saved = Store::new(&pdf).position();
        assert_eq!((saved.page, saved.zoom), (1, 2.));
        let reopened = cx.add_window(|w, cx| Reader::new(pdf, w, cx));
        let mut restored = VisualTestContext::from_window(reopened.into(), cx);
        restored.simulate_resize(size(px(700.), px(600.)));
        for _ in 0..300 {
            restored.executor().advance_clock(Duration::from_millis(15));
            restored.run_until_parked();
            restored.update(|w, cx| w.draw(cx).clear(cx));
            let ready = reopened
                .update(&mut restored, |r, _, _| {
                    if !r.pages.contains_key(&1) {
                        return false;
                    }
                    assert_eq!(r.page, 1);
                    assert!(r.position.near(saved));
                    true
                })
                .unwrap();
            if ready {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("reopened PDF did not render the saved page");
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
