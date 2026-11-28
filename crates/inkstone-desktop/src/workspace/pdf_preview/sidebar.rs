use super::*;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum SidebarTab {
    #[default]
    Thumbnails,
    Outline,
    Marks,
}

type Thumbnail = Option<Result<Arc<RenderImage>, String>>;

#[derive(Default)]
pub(super) struct Sidebar {
    pub(super) open: bool,
    tab: SidebarTab,
    pub(super) thumbnails: BTreeMap<i32, Thumbnail>,
}

impl Sidebar {
    pub(super) fn clear(&mut self, cx: &mut App) {
        for (_, value) in std::mem::take(&mut self.thumbnails) {
            if let Some(Ok(image)) = value {
                cx.drop_image(image, None);
            }
        }
    }
}

impl Reader {
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        self.query.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn sidebar_view(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = div().flex().gap_2().px_2().py_1();
        for (id, label, tab) in [
            ("pdf-thumbnails", "缩略图", SidebarTab::Thumbnails),
            ("pdf-outline", "目录", SidebarTab::Outline),
            ("pdf-mark-list", "标记", SidebarTab::Marks),
        ] {
            tabs = tabs.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .cursor_pointer()
                    .when(self.sidebar.tab == tab, |el| el.underline())
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar.tab = tab;
                        cx.notify();
                    })),
            );
        }
        let panel = div()
            .flex()
            .flex_col()
            .w(px(188.))
            .min_h_0()
            .flex_shrink_0()
            .border_r_1()
            .child(tabs);
        match self.sidebar.tab {
            SidebarTab::Marks => panel.child(self.markup_list(cx)),
            SidebarTab::Outline => {
                let mut outline = div()
                    .id("pdf-outline-list")
                    .overflow_y_scroll()
                    .min_h_0()
                    .px_2();
                if let Some(layout) = &self.layout {
                    if layout.outline.is_empty() {
                        outline = outline.child("此 PDF 没有目录");
                    }
                    for (i, (title, page)) in layout.outline.iter().enumerate() {
                        let index = *page;
                        outline = outline.child(
                            div()
                                .id(("outline", i))
                                .cursor_pointer()
                                .py_1()
                                .child(title.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.navigate(index);
                                    cx.notify();
                                })),
                        );
                    }
                }
                panel.child(outline)
            }
            SidebarTab::Thumbnails => panel.child(
                uniform_list(
                    "pdf-thumbnail-list",
                    self.count as usize,
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        // GPUI also measures a single row before requesting the visible range.
                        if range != (0..1) {
                            this.sidebar.thumbnails.retain(|page, slot| {
                                if *page == 0 || range.contains(&(*page as usize)) {
                                    return true;
                                }
                                if let Some(Ok(image)) = slot {
                                    cx.drop_image(image.clone(), None);
                                }
                                false
                            });
                        }
                        range
                            .map(|i| {
                                let page = i as i32;
                                let slot =
                                    this.sidebar.thumbnails.entry(page).or_insert_with(|| {
                                        let _ = this.requests.send(Command::Thumbnail(page));
                                        None
                                    });
                                let mut preview = div()
                                    .h(px(100.))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center();
                                match slot {
                                    Some(Ok(image)) => {
                                        preview = preview.child(
                                            img(image.clone())
                                                .size_full()
                                                .object_fit(ObjectFit::Contain),
                                        )
                                    }
                                    Some(Err(_)) => preview = preview.child("预览失败"),
                                    None => {}
                                }
                                div()
                                    .id(("thumbnail", i))
                                    .debug_selector(move || format!("pdf-thumbnail-{i}"))
                                    .h(px(130.))
                                    .px_2()
                                    .py_1()
                                    .cursor_pointer()
                                    .child(preview)
                                    .child(format!("第 {} 页", i + 1))
                                    .when(this.page == page, |el| el.underline())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.navigate(page);
                                        cx.notify();
                                    }))
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .min_h_0(),
            ),
        }
    }
}
