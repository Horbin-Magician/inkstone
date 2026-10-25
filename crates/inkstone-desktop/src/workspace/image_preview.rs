//! Read-only image tabs, with cursor-anchored wheel zoom and drag panning.
use super::*;
use gpui_component::button::{Button, ButtonVariants};
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
                    .child(format!("{:.0}%", self.zoom * 100.))
                    .children(
                        [
                            ("image-zoom-out", "缩小", 0.8),
                            ("image-fit", "适应窗口", 0.),
                            ("image-zoom-in", "放大", 1.25),
                        ]
                        .into_iter()
                        .map(|(id, label, factor)| {
                            Button::new(id).ghost().label(label).on_click(cx.listener(
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
                    .child(div().flex_1())
                    .child("滚轮缩放 · 拖动平移 · 双击适应窗口"),
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
    pub(super) fn open_image_tab(
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
            .position(|t| t.path == path && t.image.is_some())
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
        if source(None, &path).is_some() {
            let path = self
                .vault
                .as_ref()
                .and_then(|v| path.strip_prefix(&v.root).ok())
                .map_or_else(|| path.clone(), |p| p.to_path_buf());
            self.open_image_tab(path, window, cx);
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
            self.open_image_tab(PathBuf::from(&reference.target), window, cx);
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
    use core::prelude::v1::test;

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
                let image = w.tabs[1].image.clone().unwrap();
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
                assert!(w.tabs[1].image.is_some());
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
                        .any(|t| t.path == std::path::Path::new("image.png") && t.image.is_some())
                );
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
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
