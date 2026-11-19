//! UI state for the release workflow. Workers never touch notes or UI entities.
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    spinner::Spinner,
};
use inkstone_core::updates::{self, Download, Release};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn platform_label() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macOS · Apple Silicon",
        ("macos", "x86_64") => "macOS · Intel",
        ("windows", "x86_64") => "Windows · x64",
        ("windows", "aarch64") => "Windows · ARM64",
        ("linux", "x86_64") => "Linux · x64",
        ("linux", "aarch64") => "Linux · ARM64",
        _ => std::env::consts::OS,
    }
}

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
    fn detail(&self) -> String {
        if self.error.is_some() {
            return "请检查网络连接或稍后重试".into();
        }
        match self.phase {
            Phase::Idle => format!(
                "当前版本 {} · 支持应用内下载与 SHA-256 校验",
                env!("CARGO_PKG_VERSION")
            ),
            Phase::Checking => "正在连接 GitHub Releases 获取最新版本信息…".into(),
            Phase::Current => format!("当前安装的 {} 已是最新稳定版本", env!("CARGO_PKG_VERSION")),
            Phase::Available => match &self.release {
                Some(release) => {
                    format!("新版本 v{} 已发布，可下载安装包完成更新", release.version)
                }
                None => "有新版本可供下载".into(),
            },
            Phase::Downloading if self.cancelled.load(Ordering::Relaxed) => {
                "正在中断传输并清理临时文件…".into()
            }
            Phase::Downloading => "下载完成后将自动校验文件完整性".into(),
            Phase::Ready => "安装包已通过完整性校验，可以打开安装".into(),
            Phase::Opening => "正在校验文件并启动系统安装程序…".into(),
            Phase::Opened => "请先保存笔记并退出应用，按安装界面完成更新".into(),
            Phase::Failed => "请检查网络连接后重试".into(),
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
        let light = self.ui.prefs.light;
        let palette = crate::theme::palette(light);
        let accent_tint = rgba(if light { 0x24778a18 } else { 0x72bfd01f });
        let danger_tint = rgba(if light { 0xa4354018 } else { 0xffa8ab1f });
        let hint_bg = rgba(if light { 0x00000005 } else { 0xffffff04 });

        let mut controls = div().flex().flex_wrap().items_center().gap_2();
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
                    .icon(super::ui::icon("download").size(px(15.)))
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
                    .icon(super::ui::icon("folder-open").size(px(15.)))
                    .label("显示安装包")
                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
            );
        }
        controls = controls.child(
            Button::new("check-updates")
                .icon(super::ui::icon("refresh-cw").size(px(15.)))
                .label("检查更新")
                .disabled(state.busy())
                .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx))),
        );

        let status_badge = {
            let is_error = state.phase == Phase::Failed || state.error.is_some();
            let badge_bg = if is_error { danger_tint } else { accent_tint };
            let glyph: AnyElement = match state.phase {
                Phase::Checking | Phase::Opening => Spinner::new()
                    .small()
                    .color(palette.accent.into())
                    .into_any_element(),
                _ if is_error => super::ui::icon("info")
                    .size(px(18.))
                    .text_color(palette.danger)
                    .into_any_element(),
                Phase::Idle => super::ui::icon("refresh-cw")
                    .size(px(18.))
                    .text_color(palette.accent)
                    .into_any_element(),
                Phase::Current | Phase::Ready | Phase::Opened => super::ui::icon("check")
                    .size(px(18.))
                    .text_color(palette.accent)
                    .into_any_element(),
                Phase::Available => super::ui::icon("sparkles")
                    .size(px(18.))
                    .text_color(palette.accent)
                    .into_any_element(),
                Phase::Downloading => super::ui::icon("download")
                    .size(px(18.))
                    .text_color(palette.accent)
                    .into_any_element(),
                Phase::Failed => super::ui::icon("info")
                    .size(px(18.))
                    .text_color(palette.danger)
                    .into_any_element(),
            };
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(px(38.))
                .rounded(px(10.))
                .bg(badge_bg)
                .child(glyph)
        };

        let status_color = if state.phase == Phase::Failed || state.error.is_some() {
            palette.danger
        } else if state.phase == Phase::Available {
            palette.accent
        } else {
            palette.foreground
        };

        let hero_card = div()
            .flex()
            .items_center()
            .gap_5()
            .px(px(24.))
            .py(px(22.))
            .rounded(px(14.))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .child(
                img(super::welcome::app_icon())
                    .size(px(64.))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_height(relative(1.2))
                                    .text_color(palette.foreground)
                                    .child(crate::product::name().to_string()),
                            )
                            .child(
                                div()
                                    .px(px(9.))
                                    .py(px(2.))
                                    .rounded_full()
                                    .bg(accent_tint)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.accent)
                                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .px(px(9.))
                                    .py(px(2.))
                                    .rounded_full()
                                    .bg(palette.sidebar)
                                    .border_1()
                                    .border_color(palette.border)
                                    .text_size(px(12.))
                                    .text_color(palette.muted)
                                    .child(platform_label()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .line_height(relative(1.5))
                            .text_color(palette.muted)
                            .child("本地优先的 Markdown 桌面笔记应用"),
                    ),
            );

        let update_card = div()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_4()
                    .px(px(20.))
                    .py(px(18.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .flex_1()
                            .min_w(px(220.))
                            .child(status_badge)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .id("update-status")
                                            .debug_selector(|| "update-status".into())
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .line_height(relative(1.35))
                                            .text_color(status_color)
                                            .child(state.label()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(relative(1.45))
                                            .text_color(palette.muted)
                                            .child(state.detail()),
                                    ),
                            ),
                    )
                    .child(super::focus_reveal::FocusReveal::new(
                        "update-actions-focus",
                        &self.ui.settings_scroll,
                        controls,
                    )),
            )
            .when(state.phase == Phase::Downloading, |s| {
                let total = state.total.load(Ordering::Relaxed);
                let percent = if total == 0 {
                    0.
                } else {
                    (state.progress.load(Ordering::Relaxed) as f32 * 100. / total as f32)
                        .clamp(0., 100.)
                };
                s.child(
                    div()
                        .px(px(20.))
                        .pb(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(12.))
                                .text_color(palette.muted)
                                .child(if total == 0 {
                                    "正在连接下载服务器…"
                                } else {
                                    "下载进度"
                                })
                                .when(total > 0, |row| row.child(format!("{percent:.0}%"))),
                        )
                        .child(
                            gpui_component::progress::Progress::new("update-progress")
                                .w_full()
                                .loading(total == 0)
                                .value(percent),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(px(20.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(hint_bg)
                    .child(
                        super::ui::icon("info")
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(palette.muted),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(relative(1.45))
                            .text_color(palette.muted)
                            .child(if cfg!(target_os = "macos") {
                                "下载后打开 DMG。保存笔记并退出应用，再将新版本拖入 Applications 替换旧版本。"
                            } else {
                                "下载后打开安装器。请先保存笔记，按安装界面提示退出应用并完成更新。"
                            }),
                    ),
            );

        let section = |title: &str, card: Div| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_1()
                        .mb_2()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.muted)
                        .child(title.to_string()),
                )
                .child(card)
        };

        let about_row = |title: &str, description: &str, trailing: AnyElement, divider: bool| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .py(px(14.))
                .when(divider, |row| row.border_t_1().border_color(palette.border))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .font_weight(FontWeight::MEDIUM)
                                .line_height(relative(1.35))
                                .text_color(palette.foreground)
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .line_height(relative(1.45))
                                .text_color(palette.muted)
                                .child(description.to_string()),
                        ),
                )
                .child(trailing)
        };

        let repo_url = updates::REPOSITORY.to_string();
        let releases_url = format!("{}/releases", updates::REPOSITORY);
        let about_card = div()
            .flex()
            .flex_col()
            .px_5()
            .rounded(px(12.))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .child(about_row(
                "项目主页",
                "github.com/Horbin-Magician/inkstone",
                Button::new("about-repository")
                    .ghost()
                    .small()
                    .icon(super::ui::icon("external-link").size(px(14.)))
                    .label("访问仓库")
                    .on_click(move |_, _, cx| cx.open_url(&repo_url))
                    .into_any_element(),
                false,
            ))
            .child(about_row(
                "发布说明",
                "查看历史版本与完整更新日志",
                Button::new("about-releases")
                    .ghost()
                    .small()
                    .icon(super::ui::icon("external-link").size(px(14.)))
                    .label("查看日志")
                    .on_click(move |_, _, cx| cx.open_url(&releases_url))
                    .into_any_element(),
                true,
            ))
            .child(about_row(
                "开源协议",
                "基于 Apache-2.0 许可证开源发布",
                div()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded_full()
                    .bg(palette.sidebar)
                    .border_1()
                    .border_color(palette.border)
                    .text_size(px(12.))
                    .text_color(palette.muted)
                    .child("Apache-2.0")
                    .into_any_element(),
                true,
            ));

        let mut content = div()
            .w_full()
            .max_w(px(700.))
            .mx_auto()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w_0()
            .gap_5()
            .child(self.settings_title("关于与更新"))
            .child(hero_card)
            .child(section("软件更新", update_card));

        if let Some(release) = &state.release {
            let page = release.page.clone();
            let notes_card = div()
                .flex()
                .flex_col()
                .rounded(px(12.))
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .px(px(20.))
                        .py(px(14.))
                        .border_b_1()
                        .border_color(palette.border)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.foreground)
                                        .child("新版本更新说明"),
                                )
                                .child(
                                    div()
                                        .px(px(8.))
                                        .py(px(1.5))
                                        .rounded_full()
                                        .bg(accent_tint)
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.accent)
                                        .child(format!("v{}", release.version)),
                                ),
                        )
                        .child(
                            Button::new("update-release-page")
                                .ghost()
                                .small()
                                .icon(super::ui::icon("external-link").size(px(14.)))
                                .label("在 GitHub 查看")
                                .on_click(move |_, _, cx| cx.open_url(&page)),
                        ),
                )
                .child(
                    div().id("update-notes").px(px(20.)).py(px(16.)).child(
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
            content = content.child(section("更新内容", notes_card));
        }

        content = content.child(section("关于", about_card));

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
