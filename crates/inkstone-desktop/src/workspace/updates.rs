//! UI state for the release workflow. Workers never touch notes or UI entities.
use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
};
use inkstone_core::updates::{self, Download, Release};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Opening,
    Opened,
    Failed,
}
#[derive(Default)]
pub(super) struct State {
    phase: Phase,
    release: Option<Release>,
    download: Option<Download>,
    opened: Option<PathBuf>,
    error: Option<String>,
    cancelled: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
impl State {
    fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading | Phase::Opening
        )
    }
    fn label(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        match self.phase {
            Phase::Idle => "检查是否有可用的新版本".into(),
            Phase::Checking => "正在检查更新…".into(),
            Phase::Current => "已是最新版本".into(),
            Phase::Available => "发现新版本".into(),
            Phase::Downloading if self.cancelled.load(Ordering::Relaxed) => "正在取消下载…".into(),
            Phase::Downloading => format!(
                "正在下载：{:.1} / {:.1} MB",
                self.progress.load(Ordering::Relaxed) as f64 / 1_048_576.,
                self.total.load(Ordering::Relaxed) as f64 / 1_048_576.
            ),
            Phase::Ready => "下载完成，SHA-256 校验通过".into(),
            Phase::Opening => "正在校验并打开安装包…".into(),
            Phase::Opened => "安装包已打开，请按安装界面完成更新".into(),
            Phase::Failed => "操作失败，请重试".into(),
        }
    }
}

impl Workspace {
    #[cfg(target_os = "macos")]
    pub(crate) fn show_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.settings = true;
        self.ui.settings_tab = 9;
        self.ui
            .settings_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.ui.settings_scroll.set_offset(Point::default());
        self.ui.hotkey_recording = None;
        self.ui.hotkey_recording_focus = None;
        window.focus(&self.ui.modal_focus, cx);
        if self.updates.release.is_none() {
            self.check_updates(cx);
        }
        cx.notify();
    }

    pub(super) fn poll_update_progress(&self, cx: &mut Context<Self>) {
        if self.updates.phase == Phase::Downloading {
            cx.notify();
        }
    }

    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        if self.updates.busy() {
            return;
        }
        self.updates = State::default();
        self.updates.phase = Phase::Checking;
        let task = cx
            .background_executor()
            .spawn(async { updates::check(env!("CARGO_PKG_VERSION")) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(release) => {
                        this.updates.phase = if release.is_some() {
                            Phase::Available
                        } else {
                            Phase::Current
                        };
                        this.updates.release = release;
                    }
                    Err(error) => {
                        this.updates.phase = Phase::Failed;
                        this.updates.error = Some(format!("检查失败：{error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.updates.busy() {
            return;
        }
        let Some(release) = self.updates.release.clone() else {
            return;
        };
        self.updates.phase = Phase::Downloading;
        self.updates.error = None;
        self.updates.download = None;
        self.updates.cancelled = Arc::default();
        self.updates.progress = Arc::default();
        self.updates.total = Arc::default();
        let cancelled = self.updates.cancelled.clone();
        let progress = self.updates.progress.clone();
        let total = self.updates.total.clone();
        let task = cx.background_executor().spawn(async move {
            updates::download(
                &release,
                &app_dir().join("updates"),
                &cancelled,
                |done, size| {
                    progress.store(done, Ordering::Relaxed);
                    total.store(size, Ordering::Relaxed);
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                // A cancellation after the last byte but before completion also
                // discards the package; it must never enable the open button.
                if this.updates.cancelled.load(Ordering::Relaxed) {
                    this.updates.phase = Phase::Available;
                    this.updates.error = Some("下载已取消".into());
                } else {
                    match result {
                        Ok(download) => {
                            this.updates.download = Some(download);
                            this.updates.phase = Phase::Ready;
                        }
                        Err(error) => {
                            this.updates.phase = Phase::Failed;
                            this.updates.error = Some(format!("下载失败：{error}"));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_update(&mut self, cx: &mut Context<Self>) {
        if self.updates.busy() {
            return;
        }
        let Some(download) = self.updates.download.take() else {
            return;
        };
        self.updates.phase = Phase::Opening;
        self.updates.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { updates::open_installer(download) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(path) => {
                        this.updates.phase = Phase::Opened;
                        this.updates.opened = Some(path);
                    }
                    Err(error) => {
                        this.updates.phase = Phase::Failed;
                        this.updates.error = Some(format!("打开安装包失败：{error}，请重新下载"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn update_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.updates;
        let mut controls = div().flex().flex_wrap().gap_2().child(
            Button::new("check-updates")
                .label("检查更新")
                .disabled(state.busy())
                .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx))),
        );
        if state.phase == Phase::Downloading {
            controls = controls.child(
                Button::new("cancel-update")
                    .label("取消下载")
                    .disabled(state.cancelled.load(Ordering::Relaxed))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.updates.cancelled.store(true, Ordering::Relaxed);
                        cx.notify();
                    })),
            );
        } else if state.download.is_some() {
            controls = controls.child(
                Button::new("open-update")
                    .primary()
                    .label("打开安装包")
                    .disabled(state.busy())
                    .on_click(cx.listener(|this, _, _, cx| this.open_update(cx))),
            );
        } else if state.release.is_some() {
            controls = controls.child(
                Button::new("download-update")
                    .primary()
                    .label(if matches!(state.phase, Phase::Failed | Phase::Opened) {
                        "重新下载"
                    } else {
                        "下载更新"
                    })
                    .disabled(state.busy())
                    .on_click(cx.listener(|this, _, _, cx| this.download_update(cx))),
            );
        }
        let path = state
            .download
            .as_ref()
            .map(|d| d.path.clone())
            .or_else(|| state.opened.clone());
        if let Some(path) = path {
            controls = controls.child(
                Button::new("reveal-update")
                    .label("显示安装包")
                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
            );
        }
        let mut content = div().flex().flex_col().flex_shrink_0().min_w_0().gap_4()
            .child(div().text_size(px(24.)).child(format!("{} {}", crate::product::name(), env!("CARGO_PKG_VERSION"))))
            .child(div().id("update-status").debug_selector(|| "update-status".into()).child(state.label()))
            .child(super::focus_reveal::FocusReveal::new("update-actions-focus", &self.ui.settings_scroll, controls))
            .child(div().text_size(px(13.)).child(if cfg!(target_os = "macos") {
                "下载后打开 DMG。保存笔记并退出应用，再将新版本拖入 Applications 替换旧版本。"
            } else {
                "下载后打开安装器。请先保存笔记，按安装界面提示退出应用并完成更新。"
            }));
        if let Some(release) = &state.release {
            let page = release.page.clone();
            content = content
                .child(
                    div()
                        .text_size(px(18.))
                        .child(format!("新版本 {}", release.version)),
                )
                .child(
                    Button::new("update-release-page")
                        .label("在 GitHub 查看此版本")
                        .on_click(move |_, _, cx| cx.open_url(&page)),
                )
                .child(
                    div().id("update-notes").child(
                        gpui_component::text::TextView::markdown(
                            "update-release-notes",
                            if release.notes.trim().is_empty() {
                                "此版本未提供更新说明。".to_string()
                            } else {
                                release.notes.clone()
                            },
                        )
                        .selectable(true)
                        .scrollable(false),
                    ),
                );
        }
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(self.settings_content().child(content))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn settings_show_status_and_preserve_running_download(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                w.ui.settings = true;
                w.ui.settings_tab = 9;
                w.updates.phase = Phase::Downloading;
                w.updates.progress.store(512, Ordering::Relaxed);
                w.updates.total.store(1024, Ordering::Relaxed);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("update-status").is_some());
        handle
            .update(&mut visual, |w, _, cx| {
                let token = w.updates.cancelled.clone();
                w.check_updates(cx);
                assert!(w.updates.phase == Phase::Downloading);
                assert!(Arc::ptr_eq(&token, &w.updates.cancelled));
                w.ui.settings_tab = 0;
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("update-status").is_none());
        handle
            .update(&mut visual, |w, _, _| {
                w.ui.settings_tab = 9;
                assert_eq!(w.updates.progress.load(Ordering::Relaxed), 512);
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("update-status").is_some());
    }

    #[test]
    fn only_active_operations_lock_controls_and_drop_cancels_worker() {
        let mut state = State::default();
        for phase in [Phase::Checking, Phase::Downloading, Phase::Opening] {
            state.phase = phase;
            assert!(state.busy());
        }
        for phase in [
            Phase::Idle,
            Phase::Current,
            Phase::Available,
            Phase::Ready,
            Phase::Opened,
            Phase::Failed,
        ] {
            state.phase = phase;
            assert!(!state.busy());
        }
        let token = state.cancelled.clone();
        drop(state);
        assert!(token.load(Ordering::Relaxed));
    }
}
