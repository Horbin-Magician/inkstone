//! In-app image preview, shared by rendered notes and attachment navigation.
use super::*;
use gpui_base::FocusTrapElement;
use gpui_component::button::{Button, ButtonVariants};

pub(super) struct Preview {
    source: ImageSource,
    title: String,
    zoom: f32,
    previous_focus: Option<FocusHandle>,
}

fn is_image(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| {
        matches!(
            ext.to_string_lossy().to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
        )
    })
}

impl Workspace {
    pub(super) fn open_attachment(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_image(&path) {
            let title = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            self.show_image(path.into(), title, window, cx);
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
            self.show_image(
                SharedUri::from(reference.target.clone()).into(),
                "图片预览".into(),
                window,
                cx,
            );
        } else if let Some(vault) = &self.vault {
            if let Some(path) =
                inkstone_core::rendering::asset_path(&vault.root, reference, &self.index.files)
            {
                let title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.show_image(path.into(), title, window, cx);
            } else {
                self.notifications
                    .publish("图片不存在或超出笔记库。".into());
                cx.notify();
            }
        }
    }

    fn show_image(
        &mut self,
        source: ImageSource,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.image_preview = Some(Preview {
            source,
            title,
            zoom: 1.,
            previous_focus: window.focused(cx),
        });
        window.focus(&self.ui.modal_focus, cx);
        cx.notify();
    }

    pub(super) fn close_image_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(preview) = self.ui.image_preview.take() {
            if let Some(focus) = preview.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn image_preview_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(preview) = &self.ui.image_preview else {
            return div().into_any_element();
        };
        let width = (window.viewport_size().width - px(64.)).max(px(100.));
        let height = (window.viewport_size().height - px(160.)).max(px(100.));
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_image_preview(window, cx)),
            )
            .child(
                div()
                    .id("image-preview")
                    .track_focus(&self.ui.modal_focus)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.close_image_preview(window, cx);
                            cx.stop_propagation();
                        } else {
                            this.navigate_modal_tab(event, window, cx);
                        }
                    }))
                    .w(width)
                    .flex()
                    .flex_col()
                    .p_3()
                    .gap_2()
                    .rounded_lg()
                    .bg(self.bg())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(preview.title.clone()),
                            )
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
                                            if let Some(p) = &mut this.ui.image_preview {
                                                p.zoom = if factor == 0. {
                                                    1.
                                                } else {
                                                    (p.zoom * factor).clamp(0.25, 4.)
                                                };
                                            }
                                            cx.notify();
                                        },
                                    ))
                                }),
                            )
                            .child(
                                Button::new("image-preview-close")
                                    .ghost()
                                    .label("关闭 · Esc")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_image_preview(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("image-preview-scroll")
                            .w_full()
                            .h(height)
                            .overflow_scroll()
                            .child(
                                img(preview.source.clone())
                                    .w((width - px(24.)) * preview.zoom)
                                    .h(height * preview.zoom)
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
                    .focus_trap("image-preview-focus-trap", &self.ui.modal_focus),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn image_preview_restores_underlying_modal_and_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.ui.trash_open = true;
                window.focus(&workspace.ui.modal_focus, cx);
                let focus = window.focused(cx);
                workspace.open_attachment(PathBuf::from("/tmp/example.PNG"), window, cx);
                assert!(workspace.ui.image_preview.is_some());
                assert!(workspace.modal_is_open());
                workspace.close_overlays(window, cx);
                assert!(workspace.ui.image_preview.is_none());
                assert!(workspace.ui.trash_open);
                assert_eq!(window.focused(cx), focus);
            })
            .unwrap();
    }
}
