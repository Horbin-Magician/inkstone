use super::*;

impl Reader {
    pub(super) fn page_text_boxes(
        &self,
        page: i32,
        width: f32,
        aspect: f32,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let page_bounds = Bounds::new(Point::default(), size(px(width), px(width / aspect)));
        let frame = |area: Area| {
            let rect = area.bounds(page_bounds);
            div()
                .absolute()
                .left(rect.left())
                .top(rect.top())
                .w(rect.size.width)
                .max_h(rect.size.height)
        };
        let mut boxes = Vec::new();
        for (index, mark) in self
            .annotations
            .iter()
            .enumerate()
            .filter(|(_, m)| m.page == page)
        {
            let Style::PageText(area) = mark.style else {
                continue;
            };
            if self.selected_page == page && self.text_box == Some(area) {
                continue;
            }
            boxes.push(
                frame(area)
                    .id(("page-text", index))
                    .debug_selector(move || format!("page-text-{index}"))
                    .occlude()
                    .overflow_y_scroll()
                    .cursor_text()
                    .bg(rgb(0xfffdf5))
                    .text_color(rgb(0x222222))
                    .border_l_2()
                    .border_color(rgb(mark.color.rgb()))
                    .px_1()
                    .text_size(px((width / 65.).max(8.)))
                    .child(mark.quote.clone())
                    .when(self.tool != MarkTool::Pen, |el| {
                        el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.selected_page = page;
                                this.selection = None;
                                this.edit_mark(index, w, cx);
                                this.body.update(cx, |s, cx| s.focus(w, cx));
                            }))
                    })
                    .into_any_element(),
            );
        }
        if self.selected_page == page
            && let Some(area) = self.text_box
        {
            boxes.push(
                frame(area)
                    .id("pdf-text-editor")
                    .occlude()
                    .overflow_y_scroll()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Textarea::new(&self.body).w_full())
                    .into_any_element(),
            );
        }
        boxes
    }
}
