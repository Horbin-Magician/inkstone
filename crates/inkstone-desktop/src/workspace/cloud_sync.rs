use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
};
use inkstone_core::vault::sync::{self, Phase, Progress, Settings, WebDav};
use std::sync::Mutex;

type ProgressSlot = Arc<Mutex<Option<Progress>>>;
/// Wait after a local change so a burst of saves becomes one sync.
const AUTO_SYNC_QUIET: Duration = Duration::from_secs(3);
/// Back off after a failed automatic attempt. Manual sync is not delayed.
const AUTO_SYNC_RETRY: Duration = Duration::from_secs(60);

fn progress_message(progress: &Progress) -> String {
    let phase = match progress.phase {
        Phase::Scanning => "正在扫描本地文件",
        Phase::ReadingManifest => "正在读取云端清单",
        Phase::Downloading => "正在下载",
        Phase::Uploading => "正在上传",
        Phase::Verifying => "正在校验本地文件",
        Phase::Publishing => "正在更新云端清单",
        Phase::Applying => "正在写入本地变更",
        Phase::Complete => "正在完成同步",
    };
    if progress.total == 0 {
        return format!("{phase}……");
    }
    let unit = if matches!(progress.phase, Phase::Uploading | Phase::Downloading) {
        "个内容对象"
    } else {
        "个文件"
    };
    let bytes = if matches!(progress.phase, Phase::Uploading | Phase::Downloading) {
        format!(
            "，已传输 {:.2} MiB",
            progress.bytes as f64 / (1024. * 1024.)
        )
    } else {
        String::new()
    };
    format!(
        "{phase}：{}/{} {unit}（本阶段 {}%）{bytes}",
        progress.completed,
        progress.total,
        progress.completed * 100 / progress.total
    )
}

pub(super) struct State {
    url: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    pub pending: bool,
    /// Set when a sync was queued by opening the vault or a local file change.
    automatic: bool,
    /// Local files changed while a sync was running, or the last attempt failed.
    again: bool,
    /// A quiet or retry timer is waiting to be armed on the next workspace tick.
    defer_quiet: bool,
    defer_retry: bool,
    /// Executor timer so tests can advance it and a burst of edits coalesces.
    wait: Option<Task<()>>,
    busy: bool,
    message: String,
    progress: Option<Progress>,
    progress_slot: Option<ProgressSlot>,
}
impl State {
    pub(super) fn is_busy(&self) -> bool {
        self.busy
    }

    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            url: cx.new(|cx| InputState::new(window, cx).placeholder("https://服务器/dav/笔记库/")),
            username: cx.new(|cx| InputState::new(window, cx).placeholder("用户名")),
            password: cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("密码或应用专用密码");
                input.set_masked(true, window, cx);
                input
            }),
            pending: false,
            automatic: false,
            again: false,
            defer_quiet: false,
            defer_retry: false,
            wait: None,
            busy: false,
            message: String::new(),
            progress: None,
            progress_slot: None,
        }
    }
}
impl Workspace {
    /// Start queued work before releasing the window, without waiting for debounce
    /// or retry timers. Local save checks still run in tick_cloud_sync.
    pub(super) fn prepare_cloud_sync_for_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.cloud_sync.wait = None;
        self.ui.cloud_sync.defer_quiet = false;
        self.ui.cloud_sync.defer_retry = false;
        self.tick_cloud_sync(window, cx);
    }

    fn cloud_settings_disabled(&self) -> bool {
        let state = &self.ui.cloud_sync;
        // An automatic attempt may wait for saves, a quiet period, or a retry.
        // Keep configuration and manual actions available during that wait.
        self.vault.is_none() || self.loading || state.busy || (state.pending && !state.automatic)
    }

    pub(super) fn apply_cloud_secret(
        &mut self,
        loaded: Option<Result<Option<String>, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.ui.cloud_sync;
        state.pending = false;
        state.automatic = false;
        state.again = false;
        state.defer_quiet = false;
        state.defer_retry = false;
        state.wait = None;
        state.busy = false;
        state.message.clear();
        state.progress = None;
        state.progress_slot = None;
        state.url.update(cx, |input, cx| {
            input.set_value(self.ui.prefs.webdav.url.clone(), window, cx)
        });
        state.username.update(cx, |input, cx| {
            input.set_value(self.ui.prefs.webdav.username.clone(), window, cx)
        });
        let password = match loaded {
            Some(Ok(password)) => {
                if password.is_none()
                    && !self.ui.prefs.webdav.url.trim().is_empty()
                    && !self.ui.prefs.webdav.username.trim().is_empty()
                {
                    state.message = "未找到已保存的密码，请重新输入并保存。".into();
                }
                password.unwrap_or_default()
            }
            Some(Err(error)) => {
                state.message = error;
                String::new()
            }
            None => String::new(),
        };
        state
            .password
            .update(cx, |input, cx| input.set_value(password, window, cx));
        self.schedule_auto_sync(false);
    }
    /// Arm quiet and retry timers, then resume a sync deferred by an in-flight attempt.
    pub(super) fn pump_auto_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.cloud_sync.defer_quiet {
            self.ui.cloud_sync.defer_quiet = false;
            self.ui.cloud_sync.defer_retry = false;
            self.start_auto_wait(AUTO_SYNC_QUIET, window, cx);
        } else if self.ui.cloud_sync.defer_retry && self.ui.cloud_sync.wait.is_none() {
            self.ui.cloud_sync.defer_retry = false;
            self.start_auto_wait(AUTO_SYNC_RETRY, window, cx);
        } else if self.ui.cloud_sync.again
            && !self.ui.cloud_sync.busy
            && self.ui.cloud_sync.wait.is_none()
        {
            self.ui.cloud_sync.again = false;
            self.schedule_auto_sync(false);
        }
    }
    fn start_auto_wait(&mut self, delay: Duration, window: &mut Window, cx: &mut Context<Self>) {
        // Replacing the previous task cancels it, so another edit extends the quiet period.
        self.ui.cloud_sync.wait = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.cloud_sync.wait = None;
                this.pump_auto_sync(window, cx);
                this.tick_cloud_sync(window, cx);
            });
        }));
    }
    /// Queue a sync after the vault opens, or after a local create, save, or delete.
    /// File changes wait briefly so a burst coalesces; startup does not.
    /// A manual sync already waiting is left immediate.
    pub(super) fn schedule_auto_sync(&mut self, quiet: bool) {
        if !self.ui.prefs.webdav.auto || self.ui.prefs.webdav.url.trim().is_empty() {
            return;
        }
        let state = &mut self.ui.cloud_sync;
        if state.busy {
            state.again = true;
            return;
        }
        if state.pending && !state.automatic {
            return;
        }
        state.pending = true;
        state.automatic = true;
        if quiet {
            state.defer_quiet = true;
        }
    }
    fn cloud_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message.clone();
        self.ui.cloud_sync.message = message;
        cx.notify();
    }
    fn poll_cloud_progress(&mut self, cx: &mut Context<Self>) {
        let progress = self
            .ui
            .cloud_sync
            .progress_slot
            .as_ref()
            .and_then(|slot| slot.lock().unwrap().take());
        if let Some(progress) = progress {
            self.cloud_message(progress_message(&progress), cx);
            self.ui.cloud_sync.progress = Some(progress);
        }
    }
    fn cloud_settings(&self, cx: &App) -> Settings {
        Settings {
            url: self.ui.cloud_sync.url.read(cx).value().trim().to_owned(),
            username: self.ui.cloud_sync.username.read(cx).value().to_string(),
            auto: self.ui.prefs.webdav.auto,
        }
    }
    fn save_cloud_settings(&mut self, cx: &mut Context<Self>) -> bool {
        let settings = self.cloud_settings(cx);
        // URL validation does not perform network access.
        if let Err(error) = settings.validate() {
            self.cloud_message(error.to_string(), cx);
            return false;
        }
        self.ui.prefs.webdav = settings;
        self.persist_workspace(cx);
        self.store_cloud_password(cx)
    }
    fn store_cloud_password(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(vault) = self.vault.clone() else {
            self.cloud_message("请先打开笔记库，再保存同步密码。".into(), cx);
            return false;
        };
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        let saved = match super::webdav_secret::account(&self.ui.prefs.webdav, &vault.root) {
            Ok(account) => super::webdav_secret::store(&account, &password),
            Err(error) => Err(error),
        };
        if let Err(error) = saved {
            self.cloud_message(error.to_string(), cx);
            return false;
        }
        true
    }
    fn test_cloud_connection(&mut self, cx: &mut Context<Self>) {
        if self.cloud_settings_disabled() {
            return;
        }
        if !self.save_cloud_settings(cx) {
            return;
        }
        let settings = self.ui.prefs.webdav.clone();
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        self.ui.cloud_sync.busy = true;
        self.ui.pending_file_writes += 1;
        self.cloud_message("正在测试 WebDAV 连接……".into(), cx);
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { WebDav::new(&settings, &password)?.test_connection() });
        let task = crate::background_sync::retain(task, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.ui.cloud_sync.busy = false;
                this.cloud_message(
                    match result {
                        Ok(()) => "WebDAV 连接成功。读写权限与并发保护将在同步时检查。".into(),
                        Err(error) => format!("连接失败：{error}"),
                    },
                    cx,
                );
            });
        })
        .detach();
    }
    fn request_cloud_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() || self.loading || self.ui.cloud_sync.busy {
            return;
        }
        if self.ui.cloud_sync.pending && !self.ui.cloud_sync.automatic {
            return;
        }
        if !self.save_cloud_settings(cx) {
            return;
        }
        self.ui.cloud_sync.pending = true;
        self.ui.cloud_sync.automatic = false;
        self.ui.cloud_sync.again = false;
        self.ui.cloud_sync.defer_quiet = false;
        self.ui.cloud_sync.defer_retry = false;
        self.ui.cloud_sync.wait = None;
        self.save_all(window, cx);
        self.cloud_message("等待笔记保存后开始同步……".into(), cx);
        self.tick_cloud_sync(window, cx);
    }
    pub(super) fn tick_cloud_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.cloud_sync.pending || self.ui.cloud_sync.busy || self.loading {
            return;
        }
        if self.ui.cloud_sync.automatic
            && (self.ui.cloud_sync.wait.is_some()
                || self.ui.cloud_sync.defer_quiet
                || self.ui.cloud_sync.defer_retry)
        {
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| t.save.conflict.get() || t.save.error.borrow().is_some())
            || self.ui.persist_error.is_some()
        {
            self.ui.cloud_sync.pending = false;
            self.ui.cloud_sync.automatic = false;
            self.ui.cloud_sync.defer_quiet = false;
            self.ui.cloud_sync.wait = None;
            if self.ui.prefs.webdav.auto {
                // Keep retrying after the conflict or save error is resolved.
                self.ui.cloud_sync.again = true;
                self.ui.cloud_sync.defer_retry = true;
            }
            self.cloud_message("同步未开始：请先处理笔记冲突或保存错误。".into(), cx);
            return;
        }
        if self.ui.file_operation
            || self.ui.pending_file_writes > 0
            || self.ui.persisting
            || self.refreshing
            || self
                .tabs
                .iter()
                .any(|t| t.save.saving.get() || self.has_pending_input(t.id, window, cx))
        {
            return;
        }
        self.flush_document_views(window, cx);
        if self.tabs.iter().any(|t| t.save.dirty.get()) {
            self.save_all(window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            self.ui.cloud_sync.pending = false;
            self.ui.cloud_sync.automatic = false;
            return;
        };
        let settings = self.ui.prefs.webdav.clone();
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        self.ui.cloud_sync.pending = false;
        self.ui.cloud_sync.automatic = false;
        self.ui.cloud_sync.again = false;
        self.ui.cloud_sync.defer_quiet = false;
        self.ui.cloud_sync.defer_retry = false;
        self.ui.cloud_sync.wait = None;
        self.ui.cloud_sync.busy = true;
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        self.cloud_message("正在连接并准备云端目录……".into(), cx);
        self.ui.cloud_sync.progress = None;
        let progress_slot: ProgressSlot = Arc::new(Mutex::new(None));
        self.ui.cloud_sync.progress_slot = Some(progress_slot.clone());
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let remote = WebDav::new(&settings, &password)?;
            remote.prepare()?;
            sync::synchronize_with_progress(&vault, &remote, &remote.identity(), |progress| {
                // Coalesce events so large vaults cannot flood the UI queue.
                *progress_slot.lock().unwrap() = Some(progress);
            })
        });
        let task = crate::background_sync::retain(task, cx);
        let slot = self.ui.cloud_sync.progress_slot.clone().unwrap();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let running = this
                    .update(cx, |this, cx| {
                        if this.generation != generation
                            || !this.ui.cloud_sync.busy
                            || !this
                                .ui
                                .cloud_sync
                                .progress_slot
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &slot))
                        {
                            return false;
                        }
                        this.poll_cloud_progress(cx);
                        true
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation { return; }
                this.ui.file_operation = false;
                this.ui.cloud_sync.busy = false;
                this.ui.cloud_sync.progress = None;
                this.ui.cloud_sync.progress_slot = None;
                let message = match &result {
                    Ok(report) => {
                        let conflicts = if report.conflicts.is_empty() { String::new() } else {
                            format!("；{} 项冲突已保留修改版本：{}", report.conflicts.len(), report.conflicts.join("、"))
                        };
                        format!("同步完成：上传 {} 个内容对象，下载 {} 个文件，删除 {} 个文件{conflicts}", report.uploaded, report.downloaded, report.deleted)
                    }
                    Err(error) => format!("同步未完成：{error}。已完成的传输会保留，可重试。"),
                };
                if result.is_err() && this.ui.prefs.webdav.auto {
                    this.ui.cloud_sync.again = true;
                    this.ui.cloud_sync.defer_retry = true;
                }
                this.cloud_message(message, cx);
                // Even a partially completed sync can have changed files. Refresh open
                // documents through the existing external-change/conflict mechanism.
                this.rescan = true;
                this.refresh_requested = true;
                if this.ui.cloud_sync.again {
                    this.ui.cloud_sync.again = false;
                    this.schedule_auto_sync(false);
                    this.tick_cloud_sync(window, cx);
                }
                this.refresh(window, cx);
            });
        }).detach();
    }
    pub(super) fn cloud_sync_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.cloud_sync;
        let disabled = self.cloud_settings_disabled();
        div().flex().flex_1().min_h_0().child(self.settings_nav(cx)).child(
            self.settings_content().gap_3()
                .child("云同步 · WebDAV")
                .when(self.vault.is_none(), |s| s.child("请先打开笔记库，再配置云同步。同步配置按笔记库分别保存。"))
                .child("在各设备填写同一个已存在的 WebDAV 目录；不同笔记库请使用不同目录。")
                .child("服务器目录地址")
                .child(Input::new(&state.url).disabled(disabled))
                .child("用户名")
                .child(Input::new(&state.username).disabled(disabled))
                .child("密码 / 应用专用密码")
                .child(Input::new(&state.password).disabled(disabled))
                .child("密码以明文保存在本机应用数据中，不写入笔记库，也不会上传。重启或切换回来后会自动填回。建议使用 HTTPS。")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div().flex_1().child("自动同步").child(
                                div().text_size(px(MIN_UI_FONT_SIZE)).child(
                                    "打开笔记库，以及新建、保存或删除文件后自动同步。连续修改会稍等片刻再合并同步；关闭后仅在点击“立即同步”时同步。",
                                ),
                            ),
                        )
                        .child(
                            super::ui::setting_switch("webdav-auto")
                                .accessibility_label("自动同步")
                                .checked(self.ui.prefs.webdav.auto)
                                .disabled(disabled)
                                .on_click(cx.listener(|this, enabled: &bool, window, cx| {
                                    this.ui.prefs.webdav.auto = *enabled;
                                    this.persist_workspace(cx);
                                    if *enabled {
                                        this.schedule_auto_sync(false);
                                        this.tick_cloud_sync(window, cx);
                                    } else {
                                        this.ui.cloud_sync.again = false;
                                        this.ui.cloud_sync.defer_quiet = false;
                                        this.ui.cloud_sync.defer_retry = false;
                                        this.ui.cloud_sync.wait = None;
                                        if this.ui.cloud_sync.automatic {
                                            this.ui.cloud_sync.pending = false;
                                            this.ui.cloud_sync.automatic = false;
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex().gap_2()
                    .child(Button::new("webdav-save").label("保存配置").disabled(disabled).on_click(cx.listener(|this, _, _, cx| {
                        if this.save_cloud_settings(cx) {
                            let message = if this.ui.cloud_sync.password.read(cx).value().is_empty() {
                                "同步配置已更新，已清除保存的密码。"
                            } else {
                                "同步配置已更新，密码已保存在本机。"
                            };
                            this.cloud_message(message.into(), cx);
                        }
                    })))
                    .child(Button::new("webdav-test").label("测试连接").disabled(disabled).on_click(cx.listener(|this, _, _, cx| this.test_cloud_connection(cx))))
                    .child(Button::new("webdav-sync").primary().label(if state.busy { "正在处理……" } else { "立即同步" }).disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.request_cloud_sync(window, cx)))))
                .child("双向同步笔记与附件，包含修改、重命名和删除。首次同步合并两端文件；同时修改时保留云端冲突副本，修改与删除冲突时保留修改。")
                .child("隐藏文件、空文件夹、工作区设置和历史记录不参与同步。单文件上限 128 MiB，一次下载上限 512 MiB。")
                .when(state.progress_slot.is_some(), |s| {
                    let progress = state.progress.as_ref().filter(|p| p.total > 0);
                    s.child(gpui_component::progress::Progress::new("cloud-sync-progress")
                        .w_full()
                        .accessibility_label("当前同步阶段进度")
                        .loading(progress.is_none())
                        .value(progress.map_or(0., |p| p.completed as f32 * 100. / p.total as f32)))
                })
                .when(!state.message.is_empty(), |s| s.child(state.message.clone()))
        ).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PlatformKeys;
    use core::prelude::v1::test;
    use std::{
        collections::BTreeMap,
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[gpui::test]
    fn settings_remain_usable_while_auto_sync_waits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-sync-settings-{}", std::process::id()));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.settings = true;
                w.ui.settings_tab = 8;
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1000.), px(720.)));
        for waiting in [false, true] {
            handle
                .update(&mut visual, |w, _, _| {
                    w.ui.cloud_sync.pending = waiting;
                    w.ui.cloud_sync.automatic = waiting;
                    w.ui.cloud_sync.defer_quiet = waiting;
                })
                .unwrap();
            for (index, value) in [(4, "https://example.test/dav/"), (6, "user"), (8, "secret")] {
                visual.update(|window, cx| window.draw(cx).clear(cx));
                let position = handle
                    .update(&mut visual, |w, _, _| {
                        w.ui.settings_scroll
                            .bounds_for_item(index)
                            .unwrap()
                            .center()
                    })
                    .unwrap();
                visual.simulate_click(position, Modifiers::default());
                visual.simulate_platform_keystrokes("ctrl-a");
                visual.simulate_input(value);
                handle
                    .update(&mut visual, |w, window, cx| {
                        let input = match index {
                            4 => &w.ui.cloud_sync.url,
                            6 => &w.ui.cloud_sync.username,
                            _ => &w.ui.cloud_sync.password,
                        };
                        assert!(
                            input.read(cx).focus_handle(cx).is_focused(window),
                            "input {index}, waiting={waiting}"
                        );
                        assert_eq!(input.read(cx).value().as_ref(), value);
                    })
                    .unwrap();
            }
        }
        // The switch must remain clickable so a queued attempt can be cancelled.
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let switch = handle
            .update(&mut visual, |w, _, _| {
                let row = w.ui.settings_scroll.bounds_for_item(10).unwrap();
                point(row.right() - px(16.), row.center().y)
            })
            .unwrap();
        visual.simulate_click(switch, Modifiers::default());
        handle
            .update(&mut visual, |w, _, _| {
                assert!(!w.ui.prefs.webdav.auto);
                assert!(!w.ui.cloud_sync.pending);
            })
            .unwrap();

        // Testing a corrected configuration must also bypass an automatic wait.
        let server = Server::new();
        handle
            .update(&mut visual, |w, window, cx| {
                configured(w, &server.url, true, window, cx);
                w.ui.cloud_sync.pending = true;
                w.ui.cloud_sync.automatic = true;
                w.ui.cloud_sync.defer_quiet = true;
                w.test_cloud_connection(cx);
                assert!(w.ui.cloud_sync.busy);
                assert_eq!(w.ui.prefs.webdav.url, server.url);
            })
            .unwrap();
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, _| {
                assert!(!w.ui.cloud_sync.busy);
                assert!(w.ui.cloud_sync.message.starts_with("WebDAV 连接成功"));
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn progress_updates_status_and_reset_discards_old_worker_events(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let slot = Arc::new(Mutex::new(Some(Progress {
                    phase: Phase::Uploading,
                    completed: 3,
                    total: 8,
                    bytes: 2 * 1024 * 1024,
                })));
                w.ui.cloud_sync.busy = true;
                w.ui.cloud_sync.progress_slot = Some(slot.clone());
                w.poll_cloud_progress(cx);
                assert!(w.ui.cloud_sync.message.contains("3/8"));
                assert!(w.ui.cloud_sync.message.contains("37%"));
                assert!(w.ui.cloud_sync.message.contains("2.00 MiB"));
                assert_eq!(w.status, w.ui.cloud_sync.message);
                assert!(slot.lock().unwrap().is_none());
                assert_eq!(w.ui.cloud_sync.progress.as_ref().unwrap().completed, 3);
                w.apply_cloud_secret(None, window, cx);
                *slot.lock().unwrap() = Some(Progress {
                    phase: Phase::Downloading,
                    completed: 1,
                    total: 2,
                    bytes: 128,
                });
                w.poll_cloud_progress(cx);
                assert!(w.ui.cloud_sync.message.is_empty());
                assert!(w.ui.cloud_sync.progress.is_none());
                assert!(w.ui.cloud_sync.progress_slot.is_none());
            })
            .unwrap();
    }

    struct Server {
        url: String,
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let files = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
            let storage = files.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let done = stop.clone();
            let thread = std::thread::spawn(move || {
                let mut revision = 0;
                while !done.load(Ordering::Relaxed) {
                    let Ok((mut socket, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    // macOS can inherit the listener's nonblocking mode.
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(&mut socket);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let parts: Vec<_> = line.split_whitespace().collect();
                    let (method, path) = (parts[0].to_string(), parts[1].to_string());
                    let mut headers = BTreeMap::new();
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        let (key, value) = line.split_once(':').unwrap();
                        headers.insert(key.to_lowercase(), value.trim().to_string());
                    }
                    let size = headers
                        .get("content-length")
                        .map(|n| n.parse().unwrap())
                        .unwrap_or(0);
                    let mut body = vec![0; size];
                    reader.read_exact(&mut body).unwrap();
                    let mut files = storage.lock().unwrap();
                    let mut output = vec![];
                    let status = match method.as_str() {
                        "PROPFIND" => "207 Multi-Status",
                        "MKCOL" => "201 Created",
                        "GET" => match files.get(&path) {
                            Some(b) => {
                                output = b.clone();
                                "200 OK"
                            }
                            None => "404 Not Found",
                        },
                        "PUT" => {
                            if headers.get("if-none-match").is_some_and(|v| v == "*")
                                && files.contains_key(&path)
                                || headers
                                    .get("if-match")
                                    .is_some_and(|v| *v != format!("\"{revision}\""))
                            {
                                "412 Precondition Failed"
                            } else {
                                files.insert(path.clone(), body);
                                if path.ends_with("manifest.json") {
                                    revision += 1;
                                }
                                "201 Created"
                            }
                        }
                        _ => "405 Method Not Allowed",
                    };
                    write!(socket, "HTTP/1.1 {status}\r\nETag: \"{revision}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", output.len()).unwrap();
                    socket.write_all(&output).unwrap();
                }
            });
            Self {
                url,
                files,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[gpui::test]
    fn sync_saves_drafts_downloads_refreshes_and_keeps_password_out_of_vault(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-cloud-ui-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("local")).unwrap();
        std::fs::create_dir_all(root.join("source")).unwrap();
        std::fs::write(root.join("local/note.md"), "old").unwrap();
        std::fs::write(root.join("source/remote.md"), "downloaded").unwrap();
        let source = Vault::open(root.join("source"), root.join("recovery")).unwrap();
        let settings = Settings {
            url: server.url.clone(),
            username: "user".into(),
            auto: false,
        };
        let remote = WebDav::new(&settings, "secret").unwrap();
        remote.prepare().unwrap();
        sync::synchronize(&source, &remote, &remote.identity()).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("local"), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.open_note("note.md".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.ui.cloud_sync
                    .url
                    .update(cx, |s, cx| s.set_value(server.url.clone(), window, cx));
                w.ui.cloud_sync
                    .username
                    .update(cx, |s, cx| s.set_value("user", window, cx));
                w.ui.cloud_sync.password.update(cx, |s, cx| {
                    s.set_value("never-persist-this-secret", window, cx)
                });
                w.tabs[w.active.unwrap()]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("latest draft", window, cx));
                w.request_cloud_sync(window, cx);
            })
            .unwrap();
        for _ in 0..8 {
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
        }
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(
                    !w.ui.cloud_sync.busy && !w.ui.cloud_sync.pending,
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(
                    w.ui.cloud_sync.message.starts_with("同步完成"),
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(w.ui.cloud_sync.progress.is_none());
                assert!(w.ui.cloud_sync.progress_slot.is_none());
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(!w.ui.file_operation);
                assert!(w.files.contains(&PathBuf::from("remote.md")));
                assert_eq!(
                    std::fs::read_to_string(root.join("local/note.md")).unwrap(),
                    "latest draft"
                );
                assert_eq!(
                    std::fs::read_to_string(root.join("local/remote.md")).unwrap(),
                    "downloaded"
                );
                let saved =
                    std::fs::read_to_string(root.join("local/.inkstone-workspace.json")).unwrap();
                assert!(saved.contains(&server.url));
                assert!(!saved.contains("never-persist-this-secret"));
                let secrets =
                    std::fs::read_to_string(super::app_dir().join("webdav-secrets.json")).unwrap();
                assert!(secrets.contains("never-persist-this-secret"));
                assert!(!secrets.contains(&server.url));
                let stored = crate::workspace::webdav_secret::load(
                    &w.ui.prefs.webdav,
                    w.vault.as_ref().unwrap().root.as_path(),
                )
                .unwrap();
                assert_eq!(stored.as_deref(), Some("never-persist-this-secret"));
                w.ui.cloud_sync
                    .password
                    .update(cx, |s, cx| s.set_value("", window, cx));
                assert!(w.save_cloud_settings(cx));
                assert!(
                    crate::workspace::webdav_secret::load(
                        &w.ui.prefs.webdav,
                        w.vault.as_ref().unwrap().root.as_path(),
                    )
                    .unwrap()
                    .is_none()
                );
                w.ui.persist_error = Some("disk full".into());
                w.ui.cloud_sync.pending = true;
                w.tick_cloud_sync(window, cx);
                assert!(!w.ui.cloud_sync.pending);
                assert!(w.ui.cloud_sync.message.starts_with("同步未开始"));
                w.watcher = None;
            })
            .unwrap();
        assert!(
            server
                .files
                .lock()
                .unwrap()
                .values()
                .any(|v| v == b"latest draft")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn configured(
        w: &mut Workspace,
        url: &str,
        auto: bool,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        w.ui.prefs.webdav.auto = auto;
        w.ui.cloud_sync
            .url
            .update(cx, |s, cx| s.set_value(url, window, cx));
        w.ui.cloud_sync
            .username
            .update(cx, |s, cx| s.set_value("user", window, cx));
        w.ui.cloud_sync
            .password
            .update(cx, |s, cx| s.set_value("secret", window, cx));
    }

    #[gpui::test]
    fn opening_a_configured_vault_syncs_without_a_manual_click(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-auto-open-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("local")).unwrap();
        std::fs::write(root.join("local/note.md"), "local").unwrap();
        std::fs::write(
            root.join("local/.inkstone-workspace.json"),
            serde_json::json!({"webdav": {"url": server.url, "username": "user"}}).to_string(),
        )
        .unwrap();
        let settings = Settings {
            url: server.url.clone(),
            username: "user".into(),
            auto: true,
        };
        let account =
            crate::workspace::webdav_secret::account(&settings, &root.join("local")).unwrap();
        crate::workspace::webdav_secret::store(&account, "secret").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("local"), window, cx)
            })
            .unwrap();
        for _ in 0..8 {
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
        }
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(
                    w.ui.cloud_sync.message.starts_with("同步完成"),
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(!w.ui.cloud_sync.busy);
                assert!(!w.ui.cloud_sync.pending);
                w.watcher = None;
            })
            .unwrap();
        assert!(
            server
                .files
                .lock()
                .unwrap()
                .values()
                .any(|bytes| bytes == b"local")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn saving_and_deleting_queue_one_sync_after_the_quiet_period(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-auto-change-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("local")).unwrap();
        std::fs::create_dir_all(root.join("recovery")).unwrap();
        std::fs::write(root.join("local/keep.md"), "kept").unwrap();
        std::fs::write(root.join("local/gone.md"), "gone").unwrap();
        let vault = Vault::open(root.join("local"), root.join("recovery")).unwrap();
        let settings = Settings {
            url: server.url.clone(),
            username: "user".into(),
            auto: true,
        };
        let remote = WebDav::new(&settings, "secret").unwrap();
        remote.prepare().unwrap();
        sync::synchronize(&vault, &remote, &remote.identity()).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault);
                w.ui.prefs.webdav = settings;
                w.files = vec!["gone.md".into(), "keep.md".into()];
                w.add_tab("keep.md".into(), Some("kept".into()), false, window, cx);
                w.add_tab("gone.md".into(), Some("gone".into()), false, window, cx);
                configured(w, &server.url, true, window, cx);
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.pending);
                assert!(w.ui.cloud_sync.wait.is_some());
                assert!(!w.ui.cloud_sync.busy);
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("revised", window, cx));
                w.tabs[0].save.dirty.set(true);
                w.save_pending(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.cloud_sync.pending);
                assert!(!w.ui.cloud_sync.busy);
                w.manage_named_note(w.tabs[1].id, true, String::new(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            !root.join("local/gone.md").exists(),
            "trash did not remove the note"
        );
        let before = server.files.lock().unwrap().clone();
        let before_manifest = before
            .iter()
            .find(|(path, _)| path.ends_with("manifest.json"))
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        assert!(
            before_manifest.contains("gone.md"),
            "quiet period synced too early: {before_manifest}"
        );
        handle
            .update(cx, |w, window, cx| {
                w.tick_cloud_sync(window, cx);
                assert!(w.ui.cloud_sync.pending, "quiet period should still hold");
                assert!(w.ui.cloud_sync.wait.is_some());
                assert!(!w.ui.cloud_sync.busy);
            })
            .unwrap();
        cx.executor().advance_clock(AUTO_SYNC_QUIET);
        cx.run_until_parked();
        for _ in 0..8 {
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
        }
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(
                    w.ui.cloud_sync.message.starts_with("同步完成"),
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(!w.ui.cloud_sync.busy);
                assert!(!w.ui.cloud_sync.pending);
                w.watcher = None;
            })
            .unwrap();
        let files = server.files.lock().unwrap().clone();
        assert!(files.values().any(|bytes| bytes == b"revised"));
        let manifest = files
            .iter()
            .find(|(path, _)| path.ends_with("manifest.json"))
            .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
            .unwrap_or_default();
        assert!(manifest.contains("keep.md"), "{manifest}");
        assert!(!manifest.contains("gone.md"), "{manifest}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn closing_window_starts_queued_sync_and_upload_survives_window_removal(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-sync-close-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/note.md"), "upload after close").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.prefs.webdav = Settings {
                    url: server.url.clone(),
                    username: "user".into(),
                    auto: true,
                };
                configured(w, &server.url, true, window, cx);
                w.ui.discard_workspace_on_close = true;
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.wait.is_some());
                assert!(w.request_window_close(window, cx));
                assert!(w.ui.cloud_sync.busy);
                assert!(w.ui.cloud_sync.wait.is_none());
                // Removing the native window must not cancel the detached collector.
                window.remove_window();
            })
            .unwrap();
        cx.run_until_parked();
        assert!(handle.update(cx, |_, _, _| ()).is_err());
        assert!(
            server
                .files
                .lock()
                .unwrap()
                .values()
                .any(|bytes| bytes == b"upload after close")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn active_sync_does_not_block_close_but_local_writes_and_conflicts_do(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.ui.cloud_sync.busy = true;
                w.ui.pending_file_writes = 1;
                assert!(w.request_window_close(window, cx));
                w.ui.pending_file_writes = 2;
                assert!(!w.request_window_close(window, cx));
                w.ui.pending_file_writes = 1;
                w.add_tab("note.md".into(), Some("saved".into()), false, window, cx);
                w.tabs[0].save.conflict.set(true);
                assert!(!w.request_window_close(window, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn disabled_auto_sync_waits_for_a_manual_request(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-auto-off-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.prefs.webdav = Settings {
                    url: "https://example.test/dav/".into(),
                    username: "user".into(),
                    auto: false,
                };
                w.apply_cloud_secret(Some(Ok(Some("secret".into()))), window, cx);
                assert!(!w.ui.cloud_sync.pending);
                w.schedule_auto_sync(true);
                w.tick(window, cx);
                assert!(!w.ui.cloud_sync.pending);
                assert!(!w.ui.cloud_sync.busy);
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
