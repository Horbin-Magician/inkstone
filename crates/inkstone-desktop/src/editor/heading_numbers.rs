use super::*;

#[derive(Clone)]
pub(super) struct Label {
    pub anchor: std::ops::Range<usize>,
    pub text: String,
    width: Pixels,
    font_size: f32,
    weight: FontWeight,
}

impl EditorPane {
    pub fn set_heading_numbering(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.heading_numbering != enabled {
            self.heading_numbering = enabled;
            self.context_revision += 1;
            self.last_presentation = None;
            cx.notify();
        }
    }

    pub(super) fn prepare_heading_labels(
        &mut self,
        text: &str,
        objects: &[std::ops::Range<usize>],
        typography: &mut [gpui_base::input::LineTypography],
        cx: &App,
    ) {
        self.heading_labels.clear();
        if !self.live || !self.heading_numbering {
            return;
        }
        let text_system = gpui::WindowTextSystem::new(cx.text_system().clone());
        let spans: std::collections::BTreeMap<_, _> = self
            .spans
            .iter()
            .filter(|s| s.kind == Kind::Heading)
            .map(|s| (s.source.start, s))
            .collect();
        let style_indices: std::collections::BTreeMap<_, _> = typography
            .iter()
            .enumerate()
            .map(|(i, s)| (s.source_anchor().start, i))
            .collect();
        let prefixes = inkstone_core::heading_numbers::prefixes(&self.parsed.headings);
        for (heading, prefix) in self.parsed.headings.iter().zip(prefixes) {
            if objects.iter().any(|r| r.contains(&heading.offset)) {
                continue;
            }
            let Some(span) = spans.get(&heading.offset) else {
                continue;
            };
            let anchors = span.heading_line_anchors(text);
            let Some(anchor) = anchors.first() else {
                continue;
            };
            let font_size = self.font_size
                * markdown::HEADING_SCALES[usize::from(heading.level.saturating_sub(1)).min(5)];
            let weight = if heading.level == 1 {
                FontWeight::BOLD
            } else {
                FontWeight::SEMIBOLD
            };
            let label = format!("{prefix} ");
            let width = text_system
                .shape_line(
                    label.clone().into(),
                    px(font_size),
                    &[TextRun {
                        len: label.len(),
                        font: Font {
                            family: self.text_font.clone(),
                            weight,
                            ..Default::default()
                        },
                        color: crate::theme::palette(self.light).foreground.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .width;
            for anchor in &anchors {
                if let Some(&index) = style_indices.get(&anchor.start) {
                    let style = &mut typography[index];
                    let start = text[..anchor.start].rfind('\n').map_or(0, |i| i + 1);
                    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
                    let mut quoted = false;
                    for (range, indent) in self.live_quotes.iter().zip(&mut self.live_quote_indents)
                    {
                        if start <= range.start && range.start < end {
                            quoted = true;
                            // Keep the quote rail in place when reserving number space.
                            *indent -= width;
                        }
                    }
                    *style = style.clone().with_left_padding(
                        width
                            + if quoted {
                                live_quotes::inset(self.quote_rem_size)
                            } else {
                                px(0.)
                            },
                    );
                }
            }
            let line_start = text[..anchor.start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = line_start + text[line_start..].chars().next().map_or(0, char::len_utf8);
            self.heading_labels.push(Label {
                anchor: line_start..line_end,
                text: label,
                width,
                font_size,
                weight,
            });
        }
    }
}

pub(super) fn overlay(
    editor: Entity<EditorState>,
    labels: Vec<Label>,
    font: SharedString,
    light: bool,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let mut elements = Vec::new();
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for label in labels {
                    let Some(bounds) = editor.read(cx).range_to_bounds(&label.anchor) else {
                        continue;
                    };
                    if bounds.bottom() <= viewport.top() || bounds.top() >= viewport.bottom() {
                        continue;
                    }
                    let offset = label.anchor.start;
                    let mut element = div()
                        .id(("heading-number", offset))
                        .debug_selector(move || format!("heading-number-{offset}"))
                        .w(label.width)
                        .h(bounds.size.height)
                        .flex()
                        .items_center()
                        .font_family(font.clone())
                        .text_size(px(label.font_size))
                        .font_weight(label.weight)
                        .text_color(crate::theme::palette(light).foreground)
                        .child(label.text)
                        .into_any_element();
                    element.prepaint_as_root(
                        point(bounds.left() - label.width, bounds.top()),
                        size(label.width, bounds.size.height).into(),
                        window,
                        cx,
                    );
                    elements.push(element);
                }
            });
            (elements, viewport)
        },
        |_, (mut elements, viewport), window, cx| {
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for element in &mut elements {
                    element.paint(window, cx);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}
