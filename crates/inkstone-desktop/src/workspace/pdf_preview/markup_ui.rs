use super::*;

impl Reader {
    pub(super) fn reset_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.comment
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    pub(super) fn edit_mark(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mark = &self.annotations[index];
        self.editing = Some(index);
        self.color = mark.color;
        self.comment.update(cx, |input, cx| {
            input.set_value(mark.comment.clone(), window, cx)
        });
        cx.notify();
    }

    fn save_mark(&mut self, style: Option<Style>, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.editing.map(|i| &self.annotations[i]);
        let replacement = if let Some(style) = style {
            let Some(range) = old
                .map(|m| m.bytes.clone())
                .or_else(|| self.selection.clone())
            else {
                return;
            };
            let Some(quote) = self
                .active_page()
                .and_then(|p| p.text.text.get(range.clone()))
                .filter(|text| !text.trim().is_empty())
            else {
                return;
            };
            if old.is_some_and(|m| m.page != self.selected_page || m.quote != quote) {
                return;
            }
            Some(Mark {
                page: self.selected_page,
                bytes: range,
                quote: quote.into(),
                style,
                color: self.color,
                comment: self.comment.read(cx).value().to_string(),
            })
        } else {
            if old.is_none() {
                return;
            }
            None
        };
        match self.store.change(old, replacement) {
            Ok(marks) => {
                self.annotations = marks;
                self.selection = None;
                self.reset_editor(window, cx);
                self.mark_error = None;
            }
            Err(e) => self.mark_error = Some(format!("保存标记失败：{e}")),
        }
        cx.notify();
    }

    pub(super) fn markup_bar(&self, cx: &mut Context<Self>) -> Div {
        let can_save = self.active_page().is_some_and(|page| {
            if let Some(index) = self.editing {
                let m = &self.annotations[index];
                m.page == self.selected_page
                    && page.text.text.get(m.bytes.clone()) == Some(&m.quote)
            } else {
                self.selection
                    .clone()
                    .and_then(|r| page.text.text.get(r))
                    .is_some_and(|s| !s.trim().is_empty())
            }
        });
        let mut bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .flex_shrink_0();
        for (id, label, color) in [
            ("yellow", "黄", Color::Yellow),
            ("green", "绿", Color::Green),
            ("pink", "粉", Color::Pink),
        ] {
            bar = bar.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .cursor_pointer()
                    .px_1()
                    .rounded_sm()
                    .bg(rgba(color.rgb() << 8 | 0x65))
                    .when(self.color == color, |el| {
                        el.border_1().border_color(rgb(color.rgb()))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.color = color;
                        cx.notify();
                    })),
            );
        }
        if can_save {
            bar = bar.child(Input::new(&self.comment).w(px(150.)));
            for (id, label, style) in [
                ("highlight", "高亮", Style::Highlight),
                ("underline", "下划线", Style::Underline),
            ] {
                let label = match self.editing {
                    Some(i) if self.annotations[i].style == style => format!("保存{label}"),
                    Some(_) => format!("改为{label}"),
                    None => label.to_owned(),
                };
                bar = bar.child(
                    div()
                        .id(id)
                        .debug_selector(move || id.into())
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.save_mark(Some(style), window, cx)
                        })),
                );
            }
        } else if self.editing.is_none() {
            bar = bar.child("选中文字以添加标记");
        } else if self.active_page().is_some() {
            bar = bar.child("原文已变化");
        }
        if let Some(index) = self.editing {
            if let Some((target, path)) = &self.note_target {
                let target = *target;
                bar = bar.child(
                    div()
                        .id("pdf-insert-excerpt")
                        .debug_selector(|| "pdf-insert-excerpt".into())
                        .cursor_pointer()
                        .child(format!("插入到 {}", path.display()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mark = &this.annotations[index];
                            cx.emit(Excerpt::new(target, mark));
                        })),
                );
            } else {
                bar = bar.child("打开 Markdown 笔记后可插入摘录");
            }
            bar = bar.child(
                div()
                    .id("delete-mark")
                    .debug_selector(|| "delete-mark".into())
                    .cursor_pointer()
                    .child("删除标记")
                    .on_click(cx.listener(|this, _, window, cx| this.save_mark(None, window, cx))),
            );
        }
        bar
    }

    pub(super) fn markup_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("pdf-annotations")
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .px_3()
            .when(self.annotations.is_empty(), |el| el.child("暂无标记"))
            .children(self.annotations.iter().enumerate().map(|(i, m)| {
                let style = match m.style {
                    Style::Highlight => "高亮",
                    Style::Underline => "下划线",
                };
                div()
                    .id(("pdf-mark", i))
                    .debug_selector(move || format!("pdf-mark-{i}"))
                    .cursor_pointer()
                    .border_l_2()
                    .border_color(rgb(m.color.rgb()))
                    .pl_2()
                    .my_1()
                    .child(div().truncate().child(format!(
                        "第 {} 页 · {}　{}",
                        i64::from(m.page) + 1,
                        style,
                        m.quote
                    )))
                    .when(!m.comment.is_empty(), |el| {
                        el.child(div().child(m.comment.clone()))
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if (0..this.count).contains(&this.annotations[i].page) {
                            this.navigate(this.annotations[i].page);
                            this.selected_page = this.annotations[i].page;
                            this.reveal = Some(this.annotations[i].bytes.clone());
                        }
                        this.selection = None;
                        this.edit_mark(i, w, cx);
                    }))
            }))
    }
}
