//! Read-only image tabs, with cursor-anchored wheel zoom and drag panning.
use super::ui::tool;
use super::*;
use std::cell::Cell;
use std::rc::Rc;

pub(super) struct Preview {
    pub(super) source: ImageSource,
    pub focus: FocusHandle,
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<Point<Pixels>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

pub(super) fn source(vault: Option<&Vault>, path: &std::path::Path) -> Option<ImageSource> {
    let raw = path.to_string_lossy();
    if inkstone_core::rendering::is_remote_image(&raw) {
        return Some(SharedUri::from(raw.into_owned()).into());
    }
    path.extension()
        .is_some_and(|ext| {
            matches!(
                ext.to_string_lossy().to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
            )
        })
        .then(|| {
            vault
                .map_or_else(|| path.to_path_buf(), |v| v.root.join(path))
                .into()
        })
}

impl Preview {
    pub(super) fn new(source: ImageSource, cx: &mut Context<Self>) -> Self {
        Self {
            source,
            focus: cx.focus_handle(),
            zoom: 1.,
            pan: Point::default(),
            drag: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }

    fn zoom_at(&mut self, factor: f32, position: Point<Pixels>) {
        if !factor.is_finite() || factor <= 0. {
            return;
        }
        let zoom = (self.zoom * factor).clamp(0.1, 16.);
        let ratio = zoom / self.zoom;
        let anchor = position - self.bounds.get().center();
        self.pan = anchor - (anchor - self.pan) * ratio;
        self.zoom = zoom;
    }

    fn fit(&mut self) {
        self.zoom = 1.;
        self.pan = Point::default();
    }
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .id("image-tab")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .h(px(40.))
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(rems(4.5))
                            .flex_shrink_0()
                            .text_right()
                            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                            .child(format!("{:.0}%", self.zoom * 100.)),
                    )
                    .children(
                        [
                            ("image-zoom-out", "zoom-out", "缩小", 0.8),
                            ("image-fit", "scan", "适应窗口", 0.),
                            ("image-zoom-in", "zoom-in", "放大", 1.25),
                        ]
                        .into_iter()
                        .map(|(id, icon, label, factor)| {
                            tool(id, icon, label).flex_shrink_0().on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if factor == 0. {
                                        this.fit();
                                    } else {
                                        this.zoom_at(factor, this.bounds.get().center());
                                    }
                                    cx.notify();
                                },
                            ))
                        }),
                    )
                    .child(div().flex_1()),
            )
            .child(
                div()
                    .id("image-viewport")
                    .debug_selector(|| "image-viewport".into())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .cursor_move()
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        let delta = f32::from(event.delta.pixel_delta(px(40.)).y);
                        if delta.is_finite() && delta != 0. {
                            this.zoom_at((delta * 0.002).clamp(-1., 1.).exp(), event.position);
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            if event.click_count == 2 {
                                this.fit();
                            } else {
                                this.drag = Some(event.position);
                            }
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            if let Some(last) = this.drag.replace(event.position) {
                                this.pan += event.position - last;
                                cx.notify();
                            }
                        } else {
                            this.drag = None;
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .child(
                        div()
                            .absolute()
                            .w(relative(self.zoom))
                            .h(relative(self.zoom))
                            .left(relative((1. - self.zoom) / 2.))
                            .top(relative((1. - self.zoom) / 2.))
                            .ml(self.pan.x)
                            .mt(self.pan.y)
                            .child(
                                img(self.source.clone())
                                    .size_full()
                                    .object_fit(ObjectFit::Contain)
                                    .with_loading(|| {
                                        div().child("正在加载图片……").into_any_element()
                                    })
                                    .with_fallback(|| {
                                        div()
                                            .child("图片无法加载，文件可能损坏或格式不受支持。")
                                            .into_any_element()
                                    }),
                            ),
                    )
                    .child(
                        canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                            .absolute()
                            .inset_0(),
                    ),
            )
    }
}

impl Workspace {
    pub(super) fn preview_diagram(
        &mut self,
        diagram: &crate::native_graphics::Prepared,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlays(window, cx);
        let preview = cx.new(|cx| Preview::new(diagram.image.clone().into(), cx));
        // Show the inline raster immediately and sharpen the enlarged view from
        // its SVG. Limit the long edge to 4096 device pixels (64 MiB square).
        let viewport = window.viewport_size();
        let target = (f32::from(viewport.width.max(viewport.height)) * window.scale_factor() * 2.)
            .min(4096.);
        let current = diagram.image.size(0);
        if (i32::from(current.width.max(current.height)) as f32) < target {
            let svg = diagram.asset.svg.clone();
            let scale = target
                / diagram.asset.width.max(diagram.asset.height)
                / gpui::SMOOTH_SVG_SCALE_FACTOR;
            let renderer = cx.svg_renderer();
            preview.update(cx, |_, cx| {
                cx.spawn(async move |preview, cx| {
                    let image = cx
                        .background_executor()
                        .spawn(async move { renderer.render_single_frame(&svg, scale) })
                        .await;
                    if let Ok(image) = image {
                        let _ = preview.update(cx, |preview, cx| {
                            preview.source = image.into();
                            cx.notify();
                        });
                    }
                })
                .detach();
            });
        }
        self.ui.diagram_preview = Some(preview);
        window.focus(&self.ui.modal_focus, cx);
        cx.notify();
    }

    pub(super) fn open_preview_tab(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlays(window, cx);
        self.navigation_generation += 1;
        self.pending_navigation = None;
        self.views.secondary_focused = false;
        if let Some(index) = self
            .tabs
            .iter()
            .position(|t| t.path == path && t.preview.is_some())
        {
            self.focus_primary(index, window, cx);
        } else {
            self.add_tab(path, Some(String::new()), false, window, cx);
            if let Some(index) = self.active {
                self.focus_primary(index, window, cx);
            }
        }
        self.persist_workspace(cx);
    }

    pub(super) fn open_attachment(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pdf_preview::is_pdf(&path) || source(None, &path).is_some() {
            let path = self
                .vault
                .as_ref()
                .and_then(|v| path.strip_prefix(&v.root).ok())
                .map_or_else(|| path.clone(), |p| p.to_path_buf());
            self.open_preview_tab(path, window, cx);
        } else {
            cx.open_with_system(&path);
        }
    }

    pub(super) fn preview_image_reference(
        &mut self,
        reference: &inkstone_core::rendering::Reference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !reference.wiki && inkstone_core::rendering::is_remote_image(&reference.target) {
            self.open_preview_tab(PathBuf::from(&reference.target), window, cx);
        } else if let Some(vault) = &self.vault {
            if let Some(path) =
                inkstone_core::rendering::asset_path(&vault.root, reference, &self.index.files)
            {
                self.open_attachment(path, window, cx);
            } else {
                self.notifications
                    .publish("图片不存在或超出笔记库。".into());
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PlatformKeys;
    use core::prelude::v1::test;

    #[gpui::test]
    fn pdf_tabs_reuse_and_preserve_source_bytes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("vault")).unwrap();
        let vault = Vault::open(root.path().join("vault"), root.path().join("recovery")).unwrap();
        let path = PathBuf::from("paper.PDF");
        let bytes = include_bytes!("../../tests/fixtures/two-pages.pdf");
        std::fs::write(vault.root.join(&path), bytes).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.open_tree_file(path.clone(), window, cx);
                w.open_note(path.clone(), window, cx);
                assert_eq!(w.tabs.len(), 1);
                assert!(matches!(w.tabs[0].preview, Some(TabPreview::Pdf(_))));
                assert!(w.current_pane().is_none());
                assert!(!w.file_draft_snapshots(cx).contains_key(&path));
                w.save_all(window, cx);
                w.close_tab(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read(vault.root.join(path)).unwrap(), bytes);
    }

    #[gpui::test]
    fn image_tabs_reuse_close_reopen_and_never_save_binary_as_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!("inkstone-image-tabs-{}", std::process::id()));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let bytes = b"\x89PNG\r\n\x1a\n\xff\xfe\0";
        std::fs::write(vault.root.join("image.png"), bytes).unwrap();
        std::fs::write(vault.root.join("note.md"), "keep my note").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(
                    "note.md".into(),
                    Some("keep my note".into()),
                    false,
                    window,
                    cx,
                );
                let note = w.tabs[0].pane.clone();
                w.open_attachment(vault.root.join("image.png"), window, cx);
                assert_eq!(w.tabs.len(), 2);
                assert!(w.current_pane().is_none());
                assert!(!w.modal_is_open());
                let Some(TabPreview::Image(image)) = w.tabs[1].preview.clone() else {
                    panic!("expected image preview")
                };
                image.update(cx, |p, _| p.zoom = 2.);
                w.focus_primary(0, window, cx);
                w.open_attachment(vault.root.join("image.png"), window, cx);
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(image.read(cx).zoom, 2.);
                assert_eq!(w.tabs[0].pane, note);
                assert!(
                    !w.file_draft_snapshots(cx)
                        .contains_key(std::path::Path::new("image.png"))
                );
                w.save_all(window, cx);
                w.close_tab(window, cx);
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(w.current_pane(), Some(note));
                w.open_note("image.png".into(), window, cx);
                assert!(w.tabs[1].preview.is_some());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read(vault.root.join("image.png")).unwrap(), bytes);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(vault.root.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.tabs.len(), 2);
                assert!(
                    w.tabs
                        .iter()
                        .any(|t| t.path == std::path::Path::new("image.png") && t.preview.is_some())
                );
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn diagram_zoom_opens_from_reading_and_live_without_moving_the_caret(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new("escape", ClosePalette, None)]);
        });
        let source = "```mermaid\nflowchart LR\nA --> B\n```\n\ntail";
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("diagram.md".into(), Some(source.into()), false, window, cx);
                w.current_pane().unwrap().update(cx, |pane, cx| {
                    pane.live = true;
                    pane.editor.update(cx, |s, cx| {
                        s.set_selected_range(source.len()..source.len(), cx)
                    });
                });
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1000.), px(700.)));
        for reading in [true, false] {
            handle
                .update(&mut visual, |w, _, cx| {
                    w.current_pane().unwrap().update(cx, |pane, cx| {
                        pane.reading = reading;
                        cx.notify();
                    });
                })
                .unwrap();
            for _ in 0..8 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            let diagram = visual.debug_bounds("native-graphic-image-0").unwrap();
            for hovered in [false, true, false, true] {
                let position = if hovered {
                    diagram.center()
                } else {
                    point(px(1.), px(1.))
                };
                visual.simulate_mouse_move(position, None, Modifiers::default());
                visual.update(|window, cx| window.draw(cx).clear(cx));
                assert_eq!(
                    visual.debug_bounds("diagram-zoom-0").is_some(),
                    hovered,
                    "zoom control visibility, reading={reading}"
                );
            }
            let button = visual.debug_bounds("diagram-zoom-0").unwrap();
            visual.simulate_click(button.center(), Modifiers::default());
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let viewport = visual
                .debug_bounds("image-viewport")
                .expect("diagram preview");
            assert!(viewport.size.width > px(800.) && viewport.size.height > px(400.));
            visual.simulate_event(ScrollWheelEvent {
                position: viewport.center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
                ..Default::default()
            });
            handle
                .update(&mut visual, |w, _, cx| {
                    assert!(w.ui.diagram_preview.as_ref().unwrap().read(cx).zoom > 1.);
                })
                .unwrap();
            visual.simulate_platform_keystrokes("escape");
            handle
                .update(&mut visual, |w, _, cx| {
                    assert!(!w.modal_is_open());
                    let pane = w.current_pane().unwrap();
                    let editor = pane.read(cx).editor.read(cx);
                    assert_eq!(editor.value().as_ref(), source);
                    assert_eq!(editor.selected_range(), source.len()..source.len());
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn image_wheel_zoom_is_cursor_anchored_and_drag_pans(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|_, cx| Preview::new(PathBuf::from("/missing.png").into(), cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let rect = visual.debug_bounds("image-viewport").unwrap();
        let position = rect.center() + point(px(80.), px(40.));
        visual.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(120.))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        handle
            .update(&mut visual, |p, _, _| {
                assert!(p.zoom > 1.);
                let mapped = point(px(80.), px(40.)) * p.zoom + p.pan;
                assert!((mapped.x - px(80.)).abs() < px(0.01));
                assert!((mapped.y - px(40.)).abs() < px(0.01));
            })
            .unwrap();
        visual.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        let before = handle.update(&mut visual, |p, _, _| p.pan).unwrap();
        visual.simulate_mouse_move(
            position + point(px(30.), px(20.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        handle
            .update(&mut visual, |p, _, _| {
                assert_eq!(p.pan, before + point(px(30.), px(20.)));
                p.zoom_at(1e10, position);
                assert_eq!(p.zoom, 16.);
                p.fit();
                assert_eq!(p.zoom, 1.);
                assert_eq!(p.pan, Point::default());
            })
            .unwrap();
    }
}
