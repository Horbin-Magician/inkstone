use super::*;
use inkstone_core::{
    preview::{Fragment, Role},
    rendering::ReadingDocument,
};
use std::ops::Range;

#[derive(Clone)]
pub(super) struct Widget {
    pub source: Range<usize>,
    block: bool,
    role: Role,
    numbers: std::collections::BTreeMap<String, usize>,
    targets: Vec<(usize, PathBuf, usize)>,
    formula: Option<(inkstone_core::graphics::Kind, String)>,
    measured: Option<crate::native_graphics::MeasuredGraphic>,
    graphic: Option<crate::native_graphics::GraphicResult>,
    graphic_source: Option<SharedString>,
    raster_font: f32,
    raster_dpi: f32,
    pub width: f32,
    pub height: f32,
    view: Entity<TextViewState>,
    document: Arc<ReadingDocument>,
    _observer: Rc<Subscription>,
}

impl EditorPane {
    pub(super) fn install_live_objects(
        &mut self,
        fragments: Vec<(Fragment, Option<crate::native_graphics::MeasuredGraphic>)>,
        cx: &mut Context<Self>,
    ) {
        let service = crate::native_graphics::Service::get(cx);
        let mut old = std::mem::take(&mut self.live_objects);
        self.live_objects = fragments
            .into_iter()
            .map(
                |(
                    Fragment {
                        candidate,
                        document,
                        numbers,
                        targets,
                        graphic,
                    },
                    measured,
                )| {
                    let previous = old
                        .iter()
                        .position(|w| {
                            w.document.markdown == document.markdown
                                && w.block == candidate.block
                                && w.role == candidate.role
                        })
                        .map(|i| old.remove(i));
                    let reuse_geometry = previous.as_ref().is_some_and(|w| {
                        let same_measure = match (&w.measured, &measured) {
                            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                            (None, None) => true,
                            _ => false,
                        };
                        // Reparsing an unchanged formula can rebuild the measurement
                        // pointer. Keep the laid-out width and baseline height.
                        same_measure
                            || (w.formula.is_some()
                                && w.formula == graphic
                                && measured.as_ref().is_some_and(|m| m.is_ok()))
                    });
                    let same_raster = previous.as_ref().is_some_and(|w| {
                        w.raster_font.to_bits() == self.font_size.to_bits()
                            && w.raster_dpi.to_bits() == self.graphic_dpi.to_bits()
                    });
                    let (view, observer, mut width, mut height, mut raster) =
                        if let Some(previous) = previous {
                            (
                                previous.view.clone(),
                                previous._observer.clone(),
                                previous.width,
                                previous.height,
                                (reuse_geometry && same_raster)
                                    .then_some(previous.graphic.clone())
                                    .flatten(),
                            )
                        } else {
                            let view = cx.new(|cx| TextViewState::markdown(&document.markdown, cx));
                            let observer = Rc::new(cx.observe(&view, |pane, _, cx| {
                                pane.last_presentation = None;
                                cx.notify();
                            }));
                            (
                                view,
                                observer,
                                self.font_size * 4.,
                                self.font_size * 1.5,
                                None,
                            )
                        };
                    if let Some((kind, source)) = graphic.as_ref()
                        && raster.is_none()
                    {
                        raster = service.cached(
                            *kind,
                            source,
                            self.light,
                            self.font_size,
                            self.graphic_dpi,
                        );
                    }
                    if !reuse_geometry
                        && let Some(Ok(asset)) = measured.as_ref().map(|m| m.as_ref().as_ref())
                    {
                        let kind = graphic
                            .as_ref()
                            .map(|(kind, _)| *kind)
                            .unwrap_or(inkstone_core::graphics::Kind::InlineMath);
                        let scale = crate::native_graphics::scaled(kind, self.font_size);
                        width = asset.width * scale;
                        height = (asset.height * scale).max(self.font_size * 1.5);
                    }
                    Widget {
                        source: candidate.source,
                        block: candidate.block,
                        role: candidate.role,
                        numbers,
                        targets,
                        formula: graphic
                            .filter(|_| measured.as_ref().is_some_and(|measured| measured.is_ok())),
                        graphic_source: measured
                            .as_ref()
                            .filter(|measured| measured.is_ok())
                            .map(|_| self.parse_source.clone()),
                        measured,
                        graphic: raster,
                        raster_font: self.font_size,
                        raster_dpi: self.graphic_dpi,
                        width,
                        height,
                        view,
                        document: Arc::new(document),
                        _observer: observer,
                    }
                },
            )
            .collect();
    }

    // Keep unchanged objects visible while the full reading document
    // is rebuilt. Revalidate syntax as well as bytes: edits outside a formula
    // can turn it into code or otherwise change its Markdown context.
    pub(super) fn retain_live_objects(&mut self, snapshot: &Arc<inkstone_core::syntax::Snapshot>) {
        let old = self.parse_source.as_ref();
        let new = snapshot.source.as_ref();
        let prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = old.as_bytes()[prefix..]
            .iter()
            .rev()
            .zip(new.as_bytes()[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let candidates = inkstone_core::preview::candidates(snapshot);
        let current_source = SharedString::from(new.to_owned());
        for widget in &mut self.live_objects {
            let current_graphic = widget
                .graphic_source
                .as_ref()
                .is_some_and(|s| s.as_ref() == old);
            let local_document = widget.formula.is_none()
                && widget.role == Role::Content
                && widget.numbers.is_empty()
                && widget.targets.is_empty()
                && widget.document.source_matches(&self.current_path, old);
            if !current_graphic && !local_document {
                continue;
            }
            let range = if widget.source.end <= prefix {
                Some(widget.source.clone())
            } else if widget.source.start >= old.len() - suffix {
                Some(
                    new.len() - (old.len() - widget.source.start)
                        ..new.len() - (old.len() - widget.source.end),
                )
            } else {
                None
            };
            if let Some(range) = range.filter(|range| {
                candidates
                    .iter()
                    .any(|c| c.source == *range && c.block == widget.block && c.role == widget.role)
            }) {
                if current_graphic {
                    widget.source = range;
                    widget.graphic_source = Some(current_source.clone());
                } else if let Some(document) = widget
                    .document
                    .rebase_local_fragment(
                        &self.current_path,
                        old,
                        snapshot.source.clone(),
                        widget.source.clone(),
                        range.clone(),
                    )
                    .or_else(|| {
                        // Images carry reference metadata, so the dependency-free
                        // rebase deliberately rejects them. Resolve just this image
                        // against the current syntax (including reference definitions)
                        // before retaining its already laid-out view.
                        if widget.block || !old.get(widget.source.clone())?.starts_with("![") {
                            return None;
                        }
                        let document = inkstone_core::rendering::reading_snapshot(
                            &self.reference_index,
                            &self.current_path,
                            snapshot.clone(),
                            range.clone(),
                        );
                        (document.markdown == widget.document.markdown
                            && document.references.len() == widget.document.references.len()
                            && document
                                .references
                                .iter()
                                .zip(&widget.document.references)
                                .all(|(a, b)| {
                                    a.from == b.from && a.target == b.target && a.wiki == b.wiki
                                }))
                        .then_some(document)
                    })
                {
                    widget.source = range;
                    widget.document = Arc::new(document);
                }
            } else {
                widget.graphic_source = None;
            }
        }
    }

    pub(super) fn active_live_objects(
        &self,
        source: &str,
        selections: &[Range<usize>],
        matches: &[Range<usize>],
    ) -> Vec<gpui_base::input::DisplayObject> {
        if !self.live || self.reading {
            return vec![];
        }
        self.live_objects
            .iter()
            .filter(|w| {
                w.role != Role::Hidden
                    && (if w.formula.is_some() {
                        w.graphic_source
                            .as_ref()
                            .is_some_and(|s| s.as_ref() == source)
                    } else {
                        w.document.source_matches(&self.current_path, source)
                    })
                    && !selections.iter().chain(matches).any(|s| {
                        if s.is_empty() {
                            w.source.start <= s.start && s.start <= w.source.end
                        } else {
                            s.start < w.source.end && w.source.start < s.end
                        }
                    })
            })
            .map(|w| gpui_base::input::DisplayObject {
                id: w.source.start as u64,
                source: w.source.clone(),
                size: size(px(w.width.max(1.)), px(w.height.max(1.))),
                baseline: None,
            })
            .collect()
    }

    pub(super) fn hidden_live_ranges(
        &self,
        source: &str,
        selections: &[Range<usize>],
        matches: &[Range<usize>],
    ) -> Vec<Range<usize>> {
        if !self.live || self.reading {
            return vec![];
        }
        self.live_objects
            .iter()
            .filter(|w| {
                w.role == Role::Hidden
                    && w.document.source_matches(&self.current_path, source)
                    && !selections.iter().chain(matches).any(|s| {
                        if s.is_empty() {
                            w.source.start <= s.start && s.start <= w.source.end
                        } else {
                            s.start < w.source.end && w.source.start < s.end
                        }
                    })
            })
            .map(|w| w.source.clone())
            .collect()
    }

    pub(crate) fn enqueue_graphic(
        &mut self,
        kind: inkstone_core::graphics::Kind,
        source: String,
        light: bool,
        font: f32,
        dpi: f32,
        cx: &mut Context<Self>,
    ) {
        let service = crate::native_graphics::Service::get(cx);
        if service.cached(kind, &source, light, font, dpi).is_some() {
            self.attach_raster(kind, &source, cx);
            return;
        }
        let Some(permit) = service.begin(kind, &source, light, font, dpi) else {
            return;
        };
        let service = service.clone();
        self.graphic_tasks.push(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn({
                    let source = source.clone();
                    async move { service.prepare(kind, &source, light, font, dpi) }
                })
                .await;
            drop(permit);
            let _ = this.update(cx, |this, cx| {
                let _ = prepared;
                this.attach_raster(kind, &source, cx);
                // A full batch may have left more visible formulas waiting.
                this.refresh_visible_graphics(cx);
            });
        }));
    }

    pub(super) fn refresh_visible_graphics(&mut self, cx: &mut Context<Self>) {
        if self.reading || !self.live || self.editor.read(cx).is_composing() {
            return;
        }
        let pending = {
            let editor = self.editor.read(cx);
            let viewport = editor.input_bounds();
            let slack = viewport.size.height.max(px(self.font_size * 8.));
            let laid_out = self.live_objects.iter().any(|widget| {
                editor
                    .display_object_bounds(widget.source.start as u64)
                    .is_some()
            });
            let mut ordinal = 0usize;
            self.live_objects
                .iter()
                .filter_map(|widget| {
                    let formula = widget.formula.clone()?;
                    if widget.graphic.is_some() {
                        return None;
                    }
                    let near = if laid_out {
                        editor
                            .display_object_bounds(widget.source.start as u64)
                            .is_some_and(|bounds| {
                                bounds.bottom() > viewport.top() - slack
                                    && bounds.top() < viewport.bottom() + slack
                            })
                    } else {
                        let index = ordinal;
                        ordinal += 1;
                        index < 12
                    };
                    near.then_some(formula)
                })
                .collect::<Vec<_>>()
        };
        let (light, font, dpi) = (self.light, self.font_size, self.graphic_dpi);
        for (kind, source) in pending {
            self.enqueue_graphic(kind, source, light, font, dpi, cx);
        }
    }

    fn attach_raster(
        &mut self,
        kind: inkstone_core::graphics::Kind,
        source: &str,
        cx: &mut Context<Self>,
    ) {
        let service = crate::native_graphics::Service::get(cx);
        let Some(prepared) =
            service.cached(kind, source, self.light, self.font_size, self.graphic_dpi)
        else {
            return;
        };
        let current = self.parse_source.clone();
        for widget in &mut self.live_objects {
            if widget
                .formula
                .as_ref()
                .is_some_and(|(widget_kind, text)| *widget_kind == kind && text == source)
                && widget
                    .graphic_source
                    .as_ref()
                    .is_some_and(|s| s.as_ref() == current.as_ref())
            {
                widget.graphic = Some(prepared.clone());
                widget.raster_font = self.font_size;
                widget.raster_dpi = self.graphic_dpi;
            }
        }
        if self.reading {
            self.preview
                .update(cx, |state, cx| state.invalidate_inline_layout(cx));
        }
        // Tables and callouts paint formulas through the reading plugin, so a
        // finished raster has to invalidate those views as well as the overlay.
        let nested: Vec<_> = self
            .live_objects
            .iter()
            .filter(|widget| widget.formula.is_none())
            .map(|widget| widget.view.clone())
            .collect();
        for view in nested {
            view.update(cx, |state, cx| state.invalidate_inline_layout(cx));
        }
        cx.notify();
    }
}

struct Appearance {
    root: PathBuf,
    files: Arc<index::Index>,
    font: f32,
    light: bool,
    strict: bool,
    font_family: SharedString,
}

// Text rows center their ascent/descent within the reserved line height.
// Reserve both sides of that baseline so fractions and deep subscripts fit.
fn inline_math_height(height: f32, baseline: f32, base_height: f32, bias: f32) -> f32 {
    base_height
        .max(2. * (baseline - bias))
        .max(2. * (height - baseline + bias))
}

fn element(
    widget: &Widget,
    pane: WeakEntity<EditorPane>,
    appearance: &Appearance,
    row_height: Pixels,
    available: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let document = widget.document.clone();
    let image_document = document.clone();
    let task_document = document.clone();
    let task_pane = pane.clone();
    let link_pane = pane.clone();
    let source_pane = pane.clone();
    let start = widget.source.start;
    let footnote = widget.role == Role::Reference;
    let targets = widget.targets.clone();
    let target_view = widget.view.clone();
    let root = appearance.root.clone();
    let files = appearance.files.clone();
    let font = appearance.font;
    let light = appearance.light;
    let sprite = widget
        .graphic
        .as_ref()
        .and_then(|g| g.as_ref().as_ref().ok());
    let sprite_element = sprite.map(|g| {
        let scale = if g.kind == inkstone_core::graphics::Kind::Mermaid {
            (f32::from(available) / g.width).min(1.)
        } else {
            1.
        };
        let image = img(g.image.clone())
            .w(px(g.width * scale))
            .h(px(g.height * scale))
            .when(
                g.kind == inkstone_core::graphics::Kind::BlockMath,
                |image| image.flex_shrink_0().mx_auto(),
            );
        if !widget.block
            && let Some(baseline) = g.baseline
        {
            // Match ShapedLine::paint: its descent is positive, unlike the
            // signed FontMetrics used by TextSystem::baseline_offset.
            let line = window.text_system().shape_line(
                " ".into(),
                px(appearance.font),
                &[TextRun {
                    len: 1,
                    font: Font {
                        family: appearance.font_family.clone(),
                        ..Default::default()
                    },
                    color: window.text_style().color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let base_height = px(appearance.font * 1.5);
            let bias = (line.ascent - line.descent) / 2.;
            let height =
                inline_math_height(g.height, baseline, f32::from(base_height), f32::from(bias));
            let top = row_height.max(px(height)) / 2. + bias - px(baseline);
            div()
                .relative()
                .w(px(g.width))
                .h(px(height))
                .child(
                    image
                        .id(("live-math-image", start))
                        .debug_selector(move || format!("live-math-image-{start}"))
                        .absolute()
                        .top(top),
                )
                .into_any_element()
        } else {
            image.into_any_element()
        }
    });
    div()
        .id(("live-object", start))
        .debug_selector(move || format!("live-object-{start}"))
        .cursor_text()
        .when(sprite.is_some() && widget.block, |view| {
            view.w_full().overflow_x_scroll()
        })
        .when(
            sprite.is_some_and(|g| g.kind == inkstone_core::graphics::Kind::BlockMath),
            |view| view.flex(),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            let _ = source_pane.update(cx, |pane, cx| {
                let target = targets.iter().find(|(offset, _, _)| {
                    target_view
                        .read(cx)
                        .bounds_for_source_offset(*offset)
                        .is_some_and(|bounds| bounds.contains(&event.position()))
                });
                if let Some((_, path, _)) = target
                    && path != &pane.current_path
                {
                    cx.emit(EditorEvent::FollowReference(
                        inkstone_core::rendering::Reference {
                            from: pane.current_path.clone(),
                            target: path.to_string_lossy().into_owned(),
                            wiki: true,
                        },
                    ));
                    return;
                }
                let offset = target.map_or(start, |(_, _, offset)| *offset);
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(offset..offset, cx);
                    state.focus(window, cx);
                });
                pane.last_presentation = None;
                if footnote || target.is_some() {
                    pane.open_footnote(window, cx);
                }
                cx.notify();
            });
        })
        .when_some(sprite_element, |view, image| view.child(image))
        .when(sprite.is_none(), |view| {
            view.child(
                TextView::new(&widget.view)
                    .font_family(appearance.font_family.clone())
                    .markdown_extensions({
                        let mut extensions = crate::native_graphics::extensions(
                            font,
                            window.scale_factor(),
                            light,
                            appearance.strict,
                            pane.clone(),
                            cx,
                        )
                        .footnote_numbers(widget.numbers.clone());
                        if let Role::Footer(ranges) = &widget.role {
                            let ranges = ranges.clone();
                            extensions = extensions
                                .block_parser_first(move |node, _| {
                                    let p = node.position()?;
                                    ranges
                                        .iter()
                                        .any(|r| r.start == p.start.offset && r.end == p.end.offset)
                                        .then(|| {
                                            gpui_base::text::MarkdownNode::new(
                                                "inkstone-hidden-block",
                                                (),
                                            )
                                            .text("")
                                        })
                                })
                                .block_renderer("inkstone-hidden-block", |_, _, _| {
                                    div().h(px(0.)).w(px(0.))
                                });
                        }
                        extensions
                    })
                    .text_size(px(font))
                    .line_height(relative(1.5))
                    .selectable(false)
                    .scrollable(false)
                    .style(
                        gpui_base::text::TextViewStyle::from_theme(&gpui_base::Theme::global(cx))
                            .with_foreground(crate::theme::palette(light).foreground.into())
                            .with_link(crate::theme::palette(light).accent.into())
                            .with_code_background(crate::theme::palette(light).surface.into())
                            .with_border(crate::theme::palette(light).border.into())
                            .with_paragraph_gap(rems(0.)),
                    )
                    .on_task_toggle(move |offset, checked, window, cx| {
                        cx.stop_propagation();
                        if let Some(target) = task_document
                            .tasks
                            .iter()
                            .find(|t| t.rendered_start == offset)
                            .cloned()
                        {
                            let _ = task_pane.update(cx, |pane, cx| {
                                pane.toggle_task(target, checked, window, cx)
                            });
                        }
                    })
                    .on_link_click(move |url, event, _, cx| {
                        if event.is_right_click() {
                            return;
                        }
                        cx.stop_propagation();
                        let new_tab = event.modifiers().secondary();
                        let _ = link_pane.update(cx, |_, cx| {
                            if let Some(reference) = url
                                .strip_prefix("inkstone-reference:")
                                .and_then(|id| id.parse::<usize>().ok())
                                .and_then(|id| document.references.get(id))
                                .cloned()
                            {
                                if !reference.wiki
                                    && inkstone_core::rendering::is_external_link(&reference.target)
                                {
                                    cx.open_url(&reference.target);
                                } else {
                                    cx.emit(if new_tab {
                                        EditorEvent::FollowReferenceInNewTab(reference)
                                    } else {
                                        EditorEvent::FollowReference(reference)
                                    });
                                }
                            } else if inkstone_core::rendering::is_external_link(url) {
                                cx.open_url(url);
                            }
                        });
                    })
                    .image_source(move |uri| {
                        if let Some(reference) = uri
                            .to_string()
                            .strip_prefix("inkstone-reference:")
                            .and_then(|id| id.parse::<usize>().ok())
                            .and_then(|id| image_document.references.get(id))
                        {
                            if !reference.wiki
                                && inkstone_core::rendering::is_remote_image(&reference.target)
                            {
                                return SharedUri::from(reference.target.clone()).into();
                            }
                            return inkstone_core::rendering::asset_path(
                                &root,
                                reference,
                                &files.files,
                            )
                            .unwrap_or_else(|| root.join(".inkstone-missing-image"))
                            .into();
                        }
                        uri.clone().into()
                    }),
            )
        })
        .into_any_element()
}

pub(super) fn overlay(
    pane: WeakEntity<EditorPane>,
    editor: Entity<EditorState>,
    widgets: Vec<Widget>,
    appearance: (PathBuf, Arc<index::Index>, f32, bool, bool, SharedString),
) -> impl IntoElement {
    let appearance = Appearance {
        root: appearance.0,
        files: appearance.1,
        font: appearance.2,
        light: appearance.3,
        strict: appearance.4,
        font_family: appearance.5,
    };
    canvas(
        move |_, window, cx| {
            let viewport = editor.read(cx).input_bounds();
            let content = editor.read(cx).text_bounds().unwrap_or(viewport);
            let mut elements = vec![];
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                for widget in &widgets {
                    let Some(bounds) = editor
                        .read(cx)
                        .display_object_bounds(widget.source.start as u64)
                    else {
                        continue;
                    };
                    if !viewport.intersects(&bounds) {
                        continue;
                    }
                    let available = (content.right() - bounds.left()).max(px(1.));
                    let row_height = editor
                        .read(cx)
                        .range_to_bounds(&(widget.source.start..widget.source.start))
                        .map_or(bounds.size.height, |row| row.size.height);
                    let mut view = element(
                        widget,
                        pane.clone(),
                        &appearance,
                        row_height,
                        available,
                        window,
                        cx,
                    );
                    let width = if widget.block {
                        AvailableSpace::Definite(available)
                    } else {
                        AvailableSpace::MaxContent
                    };
                    let mut measured =
                        view.layout_as_root(size(width, AvailableSpace::MinContent), window, cx);
                    // Inline images keep their natural size when it fits. Give
                    // oversized content a definite width so percentage image
                    // limits resolve before measuring the reserved height.
                    if !widget.block && measured.width > available {
                        view = element(
                            widget,
                            pane.clone(),
                            &appearance,
                            row_height,
                            available,
                            window,
                            cx,
                        );
                        measured = view.layout_as_root(
                            size(
                                AvailableSpace::Definite(available),
                                AvailableSpace::MinContent,
                            ),
                            window,
                            cx,
                        );
                    }
                    let next_width = f32::from(if widget.block {
                        available
                    } else {
                        measured.width.min(available)
                    })
                    .clamp(1., 8192.);
                    let next_height =
                        f32::from(measured.height).clamp(appearance.font * 1.5, 8192.);
                    if (next_width - widget.width).abs() > 0.5
                        || (next_height - widget.height).abs() > 0.5
                    {
                        let weak = pane.clone();
                        let range = widget.source.clone();
                        let text = widget.document.markdown.clone();
                        window.defer(cx, move |_, cx| {
                            let _ = weak.update(cx, |pane, cx| {
                                if pane.editor.read(cx).is_composing() {
                                    return;
                                }
                                if let Some(current) = pane
                                    .live_objects
                                    .iter_mut()
                                    .find(|w| w.source == range && w.document.markdown == text)
                                {
                                    if pane.pending_live_anchor.is_none()
                                        && pane.editor.read(cx).scroll_offset().y < px(0.)
                                    {
                                        pane.pending_live_anchor =
                                            pane.editor.read(cx).display_scroll_anchor();
                                    }
                                    current.width = next_width;
                                    current.height = next_height;
                                    pane.last_presentation = None;
                                    cx.notify();
                                }
                            });
                        });
                    }
                    // Already measured above: reusing that layout avoids a
                    // second rich-document layout on every scroll frame.
                    view.prepaint_at(bounds.origin, window, cx);
                    elements.push(view);
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn large_images_fit_live_editor_and_preserve_aspect_ratio(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!("inkstone-live-image-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(root.join("large.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="2400" height="1200"><rect width="2400" height="1200" fill="red"/></svg>"#).unwrap();
        let source = "before\n\n![[large.svg]]\n\nafter";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |pane, _, cx| {
                pane.vault_root = root.clone();
                pane.set_paths(Arc::new(vec!["large.svg".into()]));
                cx.notify();
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..12 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                let widget = &pane.live_objects[0];
                let state = pane.editor.read(cx);
                let bounds = widget.view.read(cx).bounds();
                let viewport = state.input_bounds();
                assert!(
                    bounds.size.width > px(100.),
                    "image did not load: {bounds:?}"
                );
                assert!(
                    bounds.right() <= viewport.right(),
                    "image extends beyond editor: {bounds:?} {viewport:?}"
                );
                assert!(
                    ((bounds.size.width / bounds.size.height) - 2.).abs() < 0.05,
                    "image aspect ratio changed: {bounds:?}"
                );
                assert!((widget.height - f32::from(bounds.size.height)).abs() < 1.);
                assert_eq!(state.value().as_ref(), source);
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn unchanged_images_stay_visible_during_edits_before_background_parse(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for image in [
            "![alt](image.png)",
            "![[image.png]]",
            "![alt][pic]",
            "![pic][]",
            "![pic]",
        ] {
            let source = format!("top\n\n{image}\n\ntail\n\n[pic]: image.png");
            let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
            cx.run_until_parked();
            handle
                .update(cx, |pane, window, cx| {
                    let original = pane.live_objects[0].clone();
                    // No executor yield: assert the intermediate frame, not only
                    // the eventual background result that previously hid the flash.
                    for (at_start, text) in [(true, "中文😀\n"), (true, "more "), (false, "\nend")]
                    {
                        pane.editor.update(cx, |state, cx| {
                            let at = if at_start { 0 } else { state.value().len() };
                            state.set_selected_range(at..at, cx);
                            state.replace_text_in_range(None, text, window, cx);
                        });
                        pane.update_presentation(cx);
                        let current = pane.editor.read(cx).value();
                        let objects = pane.editor.read(cx).display_objects();
                        assert_eq!(objects.len(), 1, "{image} disappeared during editing");
                        let start = current.find(image).unwrap();
                        assert_eq!(objects[0].source, start..start + image.len());
                        assert_eq!(
                            objects[0].size,
                            size(px(original.width), px(original.height))
                        );
                        assert_eq!(pane.live_objects[0].view, original.view);
                        assert!(
                            pane.live_objects[0]
                                .document
                                .source_matches(&pane.current_path, &current)
                        );
                        assert_eq!(
                            pane.live_objects[0].document.references[0].target,
                            "image.png"
                        );
                    }
                    // Entering the image source still reveals it for editing.
                    let at = pane.live_objects[0].source.start + 2;
                    pane.editor
                        .update(cx, |state, cx| state.set_selected_range(at..at, cx));
                    pane.update_presentation(cx);
                    assert!(pane.editor.read(cx).display_objects().is_empty());
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn changed_image_dependencies_and_context_do_not_retain_stale_images(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, before, after) in [
            ("top\n\n![alt][pic]\n\n[pic]: old.png", "old.png", "new.png"),
            ("top\n\n![alt](old.png)", "old.png", "new.png"),
            ("top\n\n![[old.png]]", "old.png", "new.png"),
            ("top\n\n![alt](old.png)", "top", "```"),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            cx.run_until_parked();
            handle
                .update(cx, |pane, window, cx| {
                    assert_eq!(pane.live_objects.len(), 1);
                    let start = source.find(before).unwrap();
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(start..start + before.len(), cx);
                        state.replace_text_in_range(None, after, window, cx);
                        state.set_selected_range(0..0, cx);
                    });
                    pane.update_presentation(cx);
                    assert!(
                        pane.editor.read(cx).display_objects().is_empty(),
                        "{source}"
                    );
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn ime_keeps_prepared_objects_and_hidden_rows_stable_until_commit_or_cancel(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "plain\r\n\r\n$x^2$\r\n\r\n| A | B |\r\n| --- | --- |\r\n| 中文 | 😀 |\r\n\r\n> [!note]+ title\r\n> body\r\n\r\ntail";
        for commit in [true, false] {
            let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            for _ in 0..10 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            let (objects, hidden, revision) = handle
                .update(&mut visual, |pane, _, cx| {
                    let state = pane.editor.read(cx);
                    (
                        state.display_objects().to_vec(),
                        state.concealed_lines().to_vec(),
                        pane.parse_revision,
                    )
                })
                .unwrap();
            assert_eq!(objects.len(), 3);
            assert!(!hidden.is_empty());
            for text in ["n", "ni"] {
                handle
                    .update(&mut visual, |pane, window, cx| {
                        pane.editor.update(cx, |state, cx| {
                            if !state.is_composing() {
                                state.set_selected_range(0..0, cx);
                            }
                            state.replace_and_mark_text_in_range(None, text, None, window, cx);
                        });
                    })
                    .unwrap();
                for _ in 0..3 {
                    visual.run_until_parked();
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                }
                handle
                    .update(&mut visual, |pane, _, cx| {
                        let state = pane.editor.read(cx);
                        assert!(state.is_composing());
                        assert_eq!(pane.parse_revision, revision);
                        assert_eq!(state.concealed_lines(), hidden);
                        assert_eq!(state.display_objects().len(), objects.len());
                        for (before, current) in objects.iter().zip(state.display_objects()) {
                            assert_eq!(current.id, before.id);
                            assert_eq!(current.size, before.size);
                            assert_eq!(
                                current.source,
                                before.source.start + text.len()..before.source.end + text.len()
                            );
                            assert!(state.display_object_bounds(current.id).is_some());
                        }
                    })
                    .unwrap();
            }
            handle
                .update(&mut visual, |pane, window, cx| {
                    pane.editor.update(cx, |state, cx| {
                        if commit {
                            state.replace_text_in_range(None, "你", window, cx);
                        } else {
                            state.replace_and_mark_text_in_range(None, "", None, window, cx);
                        }
                    });
                })
                .unwrap();
            for _ in 0..10 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            handle
                .update(&mut visual, |pane, window, cx| {
                    assert!(!pane.editor.read(cx).is_composing());
                    assert_eq!(pane.editor.read(cx).display_objects().len(), objects.len());
                    assert_eq!(
                        pane.editor.read(cx).value().as_ref(),
                        if commit {
                            format!("你{source}")
                        } else {
                            source.to_string()
                        }
                    );
                    if commit {
                        pane.editor.update(cx, |state, cx| {
                            state.undo(&gpui_base::input::Undo, window, cx)
                        });
                    }
                    assert_eq!(pane.editor.read(cx).value().as_ref(), source);
                })
                .unwrap();
        }
    }
    #[gpui::test]
    fn unchanged_table_keeps_projection_geometry_and_mapping_before_background_parse(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "top\n\n| A | B |\n| --- | --- |\n| 中文 | **bold** |\n\ntail";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..10 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, window, cx| {
                assert_eq!(pane.editor.read(cx).display_objects().len(), 1);
                let original = pane.live_objects[0].clone();
                for inserted in ["中文😀\n", "more "] {
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(0..0, cx);
                        state.replace_text_in_range(None, inserted, window, cx);
                    });
                    pane.update_presentation(cx);
                    let current = pane.editor.read(cx).value();
                    let shift = current.len() - source.len();
                    let objects = pane.editor.read(cx).display_objects();
                    assert_eq!(
                        objects.len(),
                        1,
                        "unchanged table must not disappear while parsing"
                    );
                    assert_eq!(
                        objects[0].source,
                        original.source.start + shift..original.source.end + shift
                    );
                    assert_eq!(
                        objects[0].size,
                        size(px(original.width), px(original.height))
                    );
                    let retained = &pane.live_objects[0];
                    assert_eq!(retained.view, original.view);
                    assert!(
                        retained
                            .document
                            .source_matches(&pane.current_path, &current)
                    );
                    for (offset, _) in source[original.source.clone()].char_indices() {
                        assert_eq!(
                            retained
                                .document
                                .output_offset(&pane.current_path, retained.source.start + offset),
                            original
                                .document
                                .output_offset(&pane.current_path, original.source.start + offset)
                        );
                    }
                }
                let table = pane.live_objects[0].source.clone();
                pane.editor.update(cx, |state, cx| {
                    let at = table.start + 2;
                    state.set_selected_range(at..at, cx);
                    state.replace_text_in_range(None, "changed", window, cx);
                    state.set_selected_range(0..0, cx);
                });
                pane.update_presentation(cx);
                assert!(
                    pane.editor.read(cx).display_objects().is_empty(),
                    "changed table cannot reuse stale content"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn table_reference_dependencies_and_syntax_changes_do_not_reuse_stale_projection(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        for (source, inserted) in [
            (
                "top\n\n| A | B |\n| --- | --- |\n| [ref] | text |\n\ntail",
                "[ref]: other.md\n\n",
            ),
            (
                "top\n\n| A | B |\n| --- | --- |\n| plain | text |\n\ntail",
                "```\n",
            ),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            cx.run_until_parked();
            handle
                .update(cx, |pane, window, cx| {
                    assert_eq!(
                        pane.active_live_objects(source, std::slice::from_ref(&(0..0)), &[])
                            .len(),
                        1
                    );
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(0..0, cx);
                        state.replace_text_in_range(None, inserted, window, cx);
                    });
                    pane.update_presentation(cx);
                    assert!(pane.editor.read(cx).display_objects().is_empty());
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn unchanged_math_stays_rendered_before_background_parse(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "top\n\n行内 $x^2$ 和 $x^2$\n\n$$\n\\frac{1}{2}\n$$\n\ntail";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..10 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, window, cx| {
                assert_eq!(pane.editor.read(cx).display_objects().len(), 3);
                let original = pane.live_objects.clone();
                // Deliberately do not yield to the background executor between edits.
                for (offset, inserted) in [(0, "中文😀"), (0, "more ")] {
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(offset..offset, cx);
                        state.replace_text_in_range(None, inserted, window, cx);
                    });
                    pane.update_presentation(cx);
                    let current = pane.editor.read(cx).value();
                    let shift = current.len() - source.len();
                    let objects = pane.editor.read(cx).display_objects();
                    assert_eq!(objects.len(), 3);
                    for (before, after) in original.iter().zip(objects) {
                        assert_eq!(
                            after.source,
                            before.source.start + shift..before.source.end + shift
                        );
                        assert_eq!(after.size, size(px(before.width), px(before.height)));
                    }
                }
                let end = pane.editor.read(cx).value().len();
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(end..end, cx);
                    state.replace_text_in_range(None, " after", window, cx);
                });
                pane.update_presentation(cx);
                assert_eq!(pane.editor.read(cx).display_objects().len(), 3);
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(0.."more 中文😀".len(), cx);
                    state.replace_text_in_range(None, "", window, cx);
                });
                pane.update_presentation(cx);
                let objects = pane.editor.read(cx).display_objects();
                assert_eq!(objects.len(), 3);
                for (before, after) in original.iter().zip(objects) {
                    assert_eq!(after.source, before.source);
                }
                let formula = pane.live_objects[0].source.clone();
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(formula.start + 1..formula.start + 1, cx);
                });
                pane.update_presentation(cx);
                assert_eq!(pane.editor.read(cx).display_objects().len(), 2);
                pane.editor.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "y", window, cx);
                    state.set_selected_range(0..0, cx);
                });
                pane.update_presentation(cx);
                // The changed formula must not reuse its old graphic.
                assert_eq!(pane.editor.read(cx).display_objects().len(), 2);
                pane.editor.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "```\n", window, cx);
                });
                pane.update_presentation(cx);
                // Unchanged bytes now inside a code fence are no longer formulas.
                assert!(pane.editor.read(cx).display_objects().is_empty());
            })
            .unwrap();
    }

    // GPUI's NoopTextSystem leaves shaped descent signed. Native backends
    // normalize it positive; preserve that distinction in this regression test.
    struct NativeDescentTextSystem;

    impl PlatformTextSystem for NativeDescentTextSystem {
        fn add_fonts(&self, fonts: Vec<std::borrow::Cow<'static, [u8]>>) -> anyhow::Result<()> {
            NoopTextSystem.add_fonts(fonts)
        }
        fn all_font_names(&self) -> Vec<String> {
            NoopTextSystem.all_font_names()
        }
        fn font_id(&self, font: &Font) -> anyhow::Result<FontId> {
            NoopTextSystem.font_id(font)
        }
        fn font_metrics(&self, font: FontId) -> FontMetrics {
            NoopTextSystem.font_metrics(font)
        }
        fn typographic_bounds(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Bounds<f32>> {
            NoopTextSystem.typographic_bounds(font, glyph)
        }
        fn advance(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Size<f32>> {
            NoopTextSystem.advance(font, glyph)
        }
        fn glyph_for_char(&self, font: FontId, ch: char) -> Option<GlyphId> {
            NoopTextSystem.glyph_for_char(font, ch)
        }
        fn glyph_raster_bounds(
            &self,
            params: &RenderGlyphParams,
        ) -> anyhow::Result<Bounds<DevicePixels>> {
            NoopTextSystem.glyph_raster_bounds(params)
        }
        fn rasterize_glyph(
            &self,
            params: &RenderGlyphParams,
            bounds: Bounds<DevicePixels>,
        ) -> anyhow::Result<(Size<DevicePixels>, Vec<u8>)> {
            NoopTextSystem.rasterize_glyph(params, bounds)
        }
        fn recommended_rendering_mode(&self, font: FontId, size: Pixels) -> TextRenderingMode {
            NoopTextSystem.recommended_rendering_mode(font, size)
        }
        fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
            let mut line = NoopTextSystem.layout_line(text, font_size, runs);
            line.descent = line.descent.abs();
            line
        }
    }

    #[gpui::test]
    fn live_math_reserves_baseline_and_retains_measured_geometry_on_reparse(
        cx: &mut TestAppContext,
    ) {
        let mut context = TestAppContext::build_with_text_system(
            cx.dispatcher.clone(),
            None,
            Arc::new(NativeDescentTextSystem),
        );
        let cx = &mut context;
        cx.update(gpui_kit::init);
        let source = "top\n\n文字 $\\frac{1}{2}$ 和 $x_{i_j}$ 重复 $\\frac{1}{2}$ 后续\n\n```mermaid\nflowchart LR\nA --> B\n```\n\ntail";
        for font_size in [14., 16., 24., 32.] {
            let handle = cx.add_window(|w, cx| {
                let mut pane = EditorPane::new(source, w, cx);
                pane.font_size = font_size;
                pane
            });
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            for _ in 0..10 {
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
            }
            let image_bounds: Vec<_> = source
                .match_indices('$')
                .step_by(2)
                .map(|(start, _)| {
                    (
                        start,
                        visual
                            .debug_bounds(format!("live-math-image-{start}").leak())
                            .unwrap(),
                    )
                })
                .collect();
            handle
                .update(&mut visual, |pane, window, cx| {
                    let line = window.text_system().shape_line(
                        "文字".into(),
                        px(pane.font_size),
                        &[TextRun {
                            len: "文字".len(),
                            font: Font {
                                family: pane.text_font.clone(),
                                ..Default::default()
                            },
                            color: window.text_style().color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    let bias = (line.ascent - line.descent) / 2.;
                    for widget in pane.live_objects.iter().filter(|w| !w.block) {
                        let graphic = widget.graphic.as_ref().unwrap().as_ref().as_ref().unwrap();
                        let row = pane
                            .editor
                            .read(cx)
                            .range_to_bounds(&(widget.source.start..widget.source.start))
                            .unwrap();
                    let image = image_bounds
                        .iter()
                        .find(|(start, _)| *start == widget.source.start)
                        .unwrap()
                        .1;
                    let text_baseline = row.origin.y
                        + (row.size.height - line.ascent - line.descent) / 2.
                        + line.ascent;
                    let image_baseline = image.origin.y + px(graphic.baseline.unwrap());
                    assert!(
                        f32::from(image_baseline - text_baseline).abs() <= 1.,
                        "{font_size}px formula baseline {image_baseline:?}, text baseline {text_baseline:?}"
                    );
                    let top = row.size.height / 2. + bias - px(graphic.baseline.unwrap());
                    // Layout snaps the reserved height to device pixels.
                    assert!(top >= px(-1.));
                    assert!(top + px(graphic.height) <= row.size.height + px(1.));
                    assert!(widget.height >= graphic.height);
                }
                let old: Vec<_> = pane
                    .live_objects
                    .iter()
                    .map(|w| (w.view.entity_id(), w.width, w.height))
                    .collect();
                let snapshot = Arc::new(inkstone_core::syntax::Snapshot::new(source));
                let reading = inkstone_core::rendering::reading_snapshot(
                    &pane.reference_index,
                    &pane.current_path,
                    snapshot.clone(),
                    0..source.len(),
                );
                let service = crate::native_graphics::Service::get(cx);
                let fragments = inkstone_core::preview::fragments(
                    &pane.reference_index,
                    &pane.current_path,
                    snapshot,
                    &reading,
                )
                .into_iter()
                .map(|fragment| {
                    let measured = fragment
                        .graphic
                        .as_ref()
                        .map(|(kind, text)| service.measure_only(*kind, text, pane.light));
                    (fragment, measured)
                })
                .collect();
                pane.install_live_objects(fragments, cx);
                assert_eq!(
                    old,
                    pane.live_objects
                        .iter()
                        .map(|w| (w.view.entity_id(), w.width, w.height))
                        .collect::<Vec<_>>()
                );
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
        }
    }

    #[gpui::test]
    fn opening_many_formulas_measures_all_and_rasters_only_the_viewport(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = (0..120)
            .map(|i| format!("第{i}行 $x^{{{i}}}$\n\n"))
            .collect::<String>();
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(640.), px(220.)));
        for _ in 0..16 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                let formulas: Vec<_> = pane
                    .live_objects
                    .iter()
                    .filter(|w| w.formula.is_some())
                    .collect();
                assert!(formulas.len() >= 100, "{}", formulas.len());
                assert!(
                    formulas
                        .iter()
                        .all(|w| w.width > 1. && w.measured.is_some())
                );
                let painted = formulas.iter().filter(|w| w.graphic.is_some()).count();
                assert!(painted >= 1, "the open viewport should still show formulas");
                assert!(
                    painted * 3 < formulas.len(),
                    "painted {painted} of {}",
                    formulas.len()
                );
                assert!(formulas.last().unwrap().graphic.is_none());
                assert!(
                    crate::native_graphics::Service::get(cx).raster_count() * 3 < formulas.len()
                );
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn live_footnotes_share_numbering_render_footer_and_open_inline_editor(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "# top\n\nA^[短] B[^named] C^[中文 **粗体**]\n\n尾部\n\n[^named]: 命名定义";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..8 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                let footer = pane
                    .live_objects
                    .iter()
                    .find(|w| matches!(w.role, Role::Footer(_)))
                    .unwrap();
                footer.view.update(cx, |s, cx| {
                    s.select_all(cx);
                    let text = s.selected_text();
                    assert!(
                        text.contains("1. 短")
                            && text.contains("2. 命名定义")
                            && text.contains("3. 中文 粗体"),
                        "{text:?}"
                    );
                    assert!(
                        !text.contains("A1"),
                        "hidden body should not repeat: {text:?}"
                    );
                });
                assert!(!pane.editor.read(cx).concealed_lines().is_empty());
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
        let start = source.find("^[短]").unwrap();
        let selector = Box::leak(format!("live-object-{start}").into_boxed_str());
        let point = visual.debug_bounds(selector).unwrap().center();
        visual.simulate_click(point, Modifiers::default());
        visual.run_until_parked();
        handle
            .update(&mut visual, |pane, _, cx| {
                assert!(pane.footnote_edit.is_some());
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn live_objects_measure_reveal_selection_and_leave_copy_and_undo_as_source(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "# 正文\n\nbefore $\\frac{1}{2}$ after\n\n| a | b |\n| --- | --- |\n| 中文 | **加粗** |\n\n> [!note]+ 提示\n> - [ ] 任务\n\n```mermaid\nflowchart TD\nA[开始] --> B[结束]\n```\n\n尾部";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..8 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let table = source.find("| a |").unwrap();
        let selector = Box::leak(format!("live-object-{table}").into_boxed_str());
        let bounds = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(bounds.center(), Modifiers::default());
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                assert_eq!(pane.editor.read(cx).selected_range(), table..table);
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
                pane.editor
                    .update(cx, |s, cx| s.set_selected_range(0..0, cx));
                pane.update_presentation(cx);
            })
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                assert_eq!(pane.live_objects.len(), 4);
                assert_eq!(pane.editor.read(cx).display_objects().len(), 4);
                for widget in &pane.live_objects {
                    assert!(
                        widget.width > 1. && widget.height >= pane.font_size * 1.5,
                        "{}x{}",
                        widget.width,
                        widget.height
                    );
                }
                assert!(pane.live_objects.iter().any(|w| w.height > 50.));
                let table = source.find("| a |").unwrap();
                pane.editor
                    .update(cx, |s, cx| s.set_selected_range(table..table, cx));
                pane.update_presentation(cx);
                assert!(
                    !pane
                        .editor
                        .read(cx)
                        .display_objects()
                        .iter()
                        .any(|o| o.source.start == table)
                );
                pane.editor
                    .update(cx, |s, cx| s.set_selected_range(0..source.len(), cx));
                pane.update_presentation(cx);
                assert!(pane.editor.read(cx).display_objects().is_empty());
                assert_eq!(pane.editor.read(cx).selected_text().to_string(), source);
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
                pane.live = false;
                pane.update_presentation(cx);
                assert!(pane.editor.read(cx).concealed_ranges().is_empty());
            })
            .unwrap();
    }
}
