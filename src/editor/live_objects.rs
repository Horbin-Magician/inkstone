use super::*;
use inkstone::{preview::Candidate, rendering::ReadingDocument};
use std::ops::Range;

#[derive(Clone)]
pub(super) struct Widget {
    pub source: Range<usize>,
    block: bool,
    pub width: f32,
    pub height: f32,
    view: Entity<TextViewState>,
    document: Arc<ReadingDocument>,
    _observer: Rc<Subscription>,
}

impl EditorPane {
    pub(super) fn install_live_objects(
        &mut self,
        fragments: Vec<(Candidate, ReadingDocument)>,
        cx: &mut Context<Self>,
    ) {
        let old = std::mem::take(&mut self.live_objects);
        self.live_objects = fragments
            .into_iter()
            .map(|(candidate, document)| {
                let previous = old.iter().find(|w| {
                    w.document.markdown == document.markdown && w.block == candidate.block
                });
                let (view, observer, width, height) = if let Some(previous) = previous {
                    (
                        previous.view.clone(),
                        previous._observer.clone(),
                        previous.width,
                        previous.height,
                    )
                } else {
                    let view = cx.new(|cx| TextViewState::markdown(&document.markdown, cx));
                    let observer = Rc::new(cx.observe(&view, |pane, _, cx| {
                        pane.last_presentation = None;
                        cx.notify();
                    }));
                    (view, observer, self.font_size * 4., self.font_size * 1.5)
                };
                Widget {
                    source: candidate.source,
                    block: candidate.block,
                    width,
                    height,
                    view,
                    document: Arc::new(document),
                    _observer: observer,
                }
            })
            .collect();
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
                w.document.source_matches(&self.current_path, source)
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
}

struct Appearance {
    root: PathBuf,
    files: Arc<index::Index>,
    font: f32,
    light: bool,
    strict: bool,
    font_family: SharedString,
}

fn element(
    widget: &Widget,
    pane: WeakEntity<EditorPane>,
    appearance: &Appearance,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let document = widget.document.clone();
    let image_document = document.clone();
    let task_document = document.clone();
    let task_pane = pane.clone();
    let link_pane = pane.clone();
    let source_pane = pane;
    let start = widget.source.start;
    let root = appearance.root.clone();
    let files = appearance.files.clone();
    let font = appearance.font;
    let light = appearance.light;
    div()
        .id(("live-object", start))
        .debug_selector(move || format!("live-object-{start}"))
        .cursor_text()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            let _ = source_pane.update(cx, |pane, cx| {
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(start..start, cx);
                    state.focus(window, cx);
                });
                pane.last_presentation = None;
                cx.notify();
            });
        })
        .child(
            TextView::new(&widget.view)
                .font_family(appearance.font_family.clone())
                .markdown_extensions(crate::native_graphics::extensions(
                    font,
                    window.scale_factor(),
                    light,
                    appearance.strict,
                    cx,
                ))
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
                        let _ = task_pane
                            .update(cx, |pane, cx| pane.toggle_task(target, checked, window, cx));
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
                                && inkstone::rendering::is_external_link(&reference.target)
                            {
                                cx.open_url(&reference.target);
                            } else {
                                cx.emit(if new_tab {
                                    EditorEvent::FollowReferenceInNewTab(reference)
                                } else {
                                    EditorEvent::FollowReference(reference)
                                });
                            }
                        } else if inkstone::rendering::is_external_link(url) {
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
                            && inkstone::rendering::is_remote_image(&reference.target)
                        {
                            return SharedUri::from(reference.target.clone()).into();
                        }
                        return inkstone::rendering::asset_path(&root, reference, &files.files)
                            .unwrap_or_else(|| root.join(".inkstone-missing-image"))
                            .into();
                    }
                    uri.clone().into()
                }),
        )
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
                    let available = (content.right() - bounds.left() - px(12.)).max(px(1.));
                    let mut view = element(widget, pane.clone(), &appearance, window, cx);
                    let width = if widget.block {
                        AvailableSpace::Definite(available)
                    } else {
                        AvailableSpace::MaxContent
                    };
                    let measured =
                        view.layout_as_root(size(width, AvailableSpace::MinContent), window, cx);
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
                                if let Some(current) = pane
                                    .live_objects
                                    .iter_mut()
                                    .find(|w| w.source == range && w.document.markdown == text)
                                {
                                    current.width = next_width;
                                    current.height = next_height;
                                    pane.last_presentation = None;
                                    cx.notify();
                                }
                            });
                        });
                    }
                    view.prepaint_as_root(
                        bounds.origin,
                        size(px(next_width), px(next_height)).into(),
                        window,
                        cx,
                    );
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
