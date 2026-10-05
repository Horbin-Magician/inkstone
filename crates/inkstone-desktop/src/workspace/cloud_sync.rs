mod run;
use run::Run;
mod schedule;
use schedule::{Pump, Schedule, WaitKind};
mod watch;
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
};
use inkstone_core::vault::sync::{self, Phase, Progress, Settings, WebDav};

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
    schedule: Schedule<Task<()>>,
    run: Option<Run>,
    message: String,
    last_success: Option<u64>,
    success_read: u64,
    progress: Option<Progress>,
    remote_check: super::remote_check::RemoteCheck,
    watch: watch::Watch,
}
impl State {
    fn scheduling_label(&self) -> &'static str {
        if self.run.as_ref().is_some_and(Run::is_cancelled) {
            "正在取消，等待已发出的请求结束"
        } else if self.schedule.busy {
            if self.schedule.again {
                "同步进行中；后续变更已排队"
            } else {
                "正在处理同步请求"
            }
        } else if self.schedule.retry_waiting() {
            "等待自动重试（间隔 60 秒）；可点击立即同步"
        } else if self.schedule.defer_quiet || self.schedule.wait.is_some() {
            "正在合并连续修改，稍后同步"
        } else if self.schedule.pending || self.schedule.again {
            "同步已排队，等待保存或文件操作完成"
        } else if self.watch.has_work() {
            "正在等待或检查外部文件变更"
        } else {
            "当前没有同步任务"
        }
    }
    pub(super) fn is_pending(&self) -> bool {
        self.schedule.pending
    }
    pub(super) fn is_busy(&self) -> bool {
        self.schedule.busy
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
            schedule: Schedule::default(),
            run: None,
            message: String::new(),
            last_success: None,
            success_read: 0,
            progress: None,
            remote_check: Default::default(),
            watch: Default::default(),
        }
    }
}
impl Workspace {
    fn cancel_running_sync(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.ui.cloud_sync.run.as_ref() else {
            return;
        };
        if run.request_cancel() {
            self.cancel_queued_sync();
            self.cloud_message("正在取消同步，等待已发出的网络请求结束……".into(), cx);
        } else {
            self.cloud_message("同步已进入提交阶段，将完成本次提交后结束。".into(), cx);
        }
    }
    /// Cancel only queued work; an active transfer keeps its lifetime/protection.
    pub(super) fn cancel_queued_sync(&mut self) {
        let state = &mut self.ui.cloud_sync;
        state.schedule.cancel_queued();
        state.remote_check = Default::default();
        state.watch.reset();
    }

    /// Start queued work before releasing the window, without waiting for debounce
    /// or retry timers. Local save checks still run in tick_cloud_sync.
    pub(super) fn prepare_cloud_sync_for_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.cloud_sync.schedule.force_ready();
        self.tick_cloud_sync(window, cx);
    }

    fn cloud_settings_disabled(&self) -> bool {
        let state = &self.ui.cloud_sync;
        // An automatic attempt may wait for saves, a quiet period, or a retry.
        // Keep configuration and manual actions available during that wait.
        self.vault.is_none() || self.loading || state.schedule.busy
    }

    pub(super) fn apply_cloud_secret(
        &mut self,
        loaded: Option<Result<Option<String>, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.ui.cloud_sync;
        state.remote_check = Default::default();
        state.watch.reset();
        state.schedule = Schedule::default();
        state.run = None;
        state.message.clear();
        state.progress = None;
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
        self.load_sync_success(cx);
        self.schedule_auto_sync(false);
    }
    fn poll_remote_checks(&mut self, now: std::time::Instant, active: bool) {
        let enabled = self.vault.is_some()
            && !self.loading
            && self.ui.prefs.webdav.auto
            && !self.ui.prefs.webdav.url.trim().is_empty();
        let state = &mut self.ui.cloud_sync;
        let queued = state.schedule.queued();
        if state.remote_check.poll(
            now,
            enabled,
            Duration::from_secs(self.ui.prefs.webdav.poll_minutes.clamp(1, 1440) * 60),
            active,
            queued,
        ) {
            self.schedule_auto_sync(false);
        }
    }
    /// Arm quiet and retry timers, then resume a sync deferred by an in-flight attempt.
    pub(super) fn pump_auto_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_remote_checks(std::time::Instant::now(), window.is_window_active());
        match self.ui.cloud_sync.schedule.pump() {
            Pump::Wait(kind) => self.start_auto_wait(kind, window, cx),
            Pump::Reschedule => self.schedule_auto_sync(false),
            Pump::Idle => {}
        }
    }

    fn start_auto_wait(&mut self, kind: WaitKind, window: &mut Window, cx: &mut Context<Self>) {
        // Replacing the previous task cancels it, so another edit extends the quiet period.
        let delay = match kind {
            WaitKind::Quiet => AUTO_SYNC_QUIET,
            WaitKind::Retry => AUTO_SYNC_RETRY,
        };
        let handle = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.cloud_sync.schedule.elapsed();
                this.pump_auto_sync(window, cx);
                this.tick_cloud_sync(window, cx);
            });
        });
        self.ui.cloud_sync.schedule.set_wait(kind, handle);
        cx.notify();
    }
    /// Queue a sync after the vault opens, or after a local create, save, or delete.
    /// File changes wait briefly so a burst coalesces; startup does not.
    /// A manual sync already waiting is left immediate.
    pub(super) fn schedule_auto_sync(&mut self, quiet: bool) {
        if !self.ui.prefs.webdav.auto || self.ui.prefs.webdav.url.trim().is_empty() {
            return;
        }
        self.ui.cloud_sync.schedule.automatic(quiet);
    }

    fn cloud_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message.clone();
        self.ui.cloud_sync.message = message;
        cx.notify();
    }
    fn poll_cloud_progress(&mut self, cx: &mut Context<Self>) {
        let progress = self.ui.cloud_sync.run.as_ref().and_then(Run::take_progress);
        if let Some(progress) = progress {
            self.cloud_message(progress_message(&progress), cx);
            self.ui.cloud_sync.progress = Some(progress);
        }
    }
    fn load_sync_success(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.ui.cloud_sync;
        state.last_success = None;
        state.success_read = state.success_read.wrapping_add(1);
        let request = state.success_read;
        let generation = self.generation;
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let settings = self.ui.prefs.webdav.clone();
        if settings.url.trim().is_empty() {
            return;
        }
        let task = cx.background_executor().spawn(async move {
            WebDav::new(&settings, "")
                .and_then(|remote| sync::last_success(&vault, &remote.identity()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.cloud_sync.success_read != request {
                    return;
                }
                match result {
                    Ok(time) => this.ui.cloud_sync.last_success = time,
                    Err(error) => this.cloud_message(format!("读取最近同步记录失败：{error}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn sync_success_label(&self) -> String {
        let date = self
            .ui
            .cloud_sync
            .last_success
            .and_then(|millis| i64::try_from(millis).ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            });
        format!(
            "最近成功同步：{}",
            date.as_deref().unwrap_or("暂无时间记录")
        )
    }
    fn cloud_settings(&self, cx: &App) -> Settings {
        Settings {
            url: self.ui.cloud_sync.url.read(cx).value().trim().to_owned(),
            username: self.ui.cloud_sync.username.read(cx).value().to_string(),
            auto: self.ui.prefs.webdav.auto,
            poll_minutes: self.ui.prefs.webdav.poll_minutes,
        }
    }
    fn save_cloud_settings(&mut self, cx: &mut Context<Self>) -> bool {
        let settings = self.cloud_settings(cx);
        // URL validation does not perform network access.
        if let Err(error) = settings.validate() {
            self.cloud_message(error.to_string(), cx);
            return false;
        }
        let account_changed = self.ui.prefs.webdav.url != settings.url
            || self.ui.prefs.webdav.username != settings.username;
        self.ui.prefs.webdav = settings;
        if account_changed {
            self.load_sync_success(cx);
        }
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
        self.ui.cloud_sync.watch.epoch = self.ui.cloud_sync.watch.epoch.wrapping_add(1);
        self.ui.cloud_sync.schedule.start_connection_test();
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
                this.ui.cloud_sync.schedule.finish();
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
        if self.vault.is_none() || self.loading || self.ui.cloud_sync.schedule.busy {
            return;
        }
        if self.ui.cloud_sync.schedule.pending && !self.ui.cloud_sync.schedule.automatic {
            return;
        }
        if !self.save_cloud_settings(cx) {
            return;
        }
        self.ui.cloud_sync.schedule.manual();
        self.cloud_message("等待笔记手动保存后开始同步……".into(), cx);
        self.tick_cloud_sync(window, cx);
    }
    pub(super) fn tick_cloud_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.cloud_sync.schedule.ready() || self.loading {
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| t.save.persistence.has_conflict() || t.save.persistence.error().is_some())
            || self.settings_save.error().is_some()
        {
            self.ui
                .cloud_sync
                .schedule
                .blocked(self.ui.prefs.webdav.auto);
            self.cloud_message("同步未开始：请先处理笔记冲突或保存错误。".into(), cx);
            return;
        }
        if self.ui.file_operation
            || self.ui.pending_file_writes > 0
            || self.settings_save.is_busy()
            || self.refreshing
            || self
                .tabs
                .iter()
                .any(|t| t.save.persistence.is_saving() || self.has_pending_input(t.id, window, cx))
        {
            return;
        }
        self.flush_document_views(window, cx);
        let waiting: std::collections::BTreeSet<_> = self
            .tabs
            .iter()
            .filter(|tab| tab.save.persistence.is_dirty())
            .map(|tab| tab.path.to_string_lossy().into_owned())
            .collect();
        if !waiting.is_empty() {
            let names = waiting
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("、");
            let message = format!(
                "同步等待手动保存 {} 篇笔记：{names}。可继续编辑或取消等待。",
                waiting.len()
            );
            if self.ui.cloud_sync.message != message {
                self.cloud_message(message, cx);
            }
            return;
        }
        let Some(vault) = self.vault.clone() else {
            self.ui.cloud_sync.schedule.clear_request();
            return;
        };
        let settings = self.ui.prefs.webdav.clone();
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        self.ui.cloud_sync.schedule.start();
        self.ui.cloud_sync.watch.epoch = self.ui.cloud_sync.watch.epoch.wrapping_add(1);
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        self.cloud_message("正在连接并准备云端目录……".into(), cx);
        self.ui.cloud_sync.progress = None;
        let run = Run::default();
        self.ui.cloud_sync.run = Some(run.clone());
        let generation = self.generation;
        let worker = run.clone();
        let task = cx.background_executor().spawn(async move {
            let remote = WebDav::new(&settings, &password)?;
            remote.prepare()?;
            sync::synchronize_cancellable(
                &vault,
                &remote,
                &remote.identity(),
                worker.cancellation(),
                |progress| worker.publish(progress),
            )
        });
        let task = crate::background_sync::retain(task, cx);
        let polling = run.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let running = this
                    .update(cx, |this, cx| {
                        if this.generation != generation
                            || !this.ui.cloud_sync.schedule.busy
                            || !this
                                .ui
                                .cloud_sync
                                .run
                                .as_ref()
                                .is_some_and(|current| current.same(&polling))
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
                if this.generation != generation || !this.ui.cloud_sync.run.as_ref().is_some_and(|current| current.same(&run)) { return; }
                let cancelled = run.is_cancelled() && result.is_err();
                this.ui.cloud_sync.run = None;
                if cancelled { this.cancel_queued_sync(); }
                this.ui.file_operation = false;
                this.ui.cloud_sync.schedule.finish();
                this.ui.cloud_sync.progress = None;
                let message = match &result {
                    Ok(report) => {
                        this.ui.cloud_sync.success_read = this.ui.cloud_sync.success_read.wrapping_add(1);
                        this.ui.cloud_sync.last_success = Some(report.completed_at_ms);
                        let conflicts = if report.conflicts.is_empty() { String::new() } else {
                            format!("；{} 项冲突已保留修改版本：{}", report.conflicts.len(), report.conflicts.join("、"))
                        };
                        format!("同步完成：上传 {} 个内容对象，下载 {} 个文件，删除 {} 个文件{conflicts}", report.uploaded, report.downloaded, report.deleted)
                    }
                    Err(_) if cancelled => "本次同步已取消。后续修改或定期检查仍可触发同步。".into(),
                    Err(error) => format!("同步未完成：{error}。已完成的传输会保留，可重试。"),
                };
                if result.is_err() && !cancelled && this.ui.prefs.webdav.auto {
                    this.ui.cloud_sync.schedule.retry();
                }
                this.cloud_message(message, cx);
                // Even a partially completed sync can have changed files. Refresh open
                // documents through the existing external-change/conflict mechanism.
                this.rescan = true;
                this.refresh_requested = true;
                if this.ui.cloud_sync.schedule.take_again() {
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
            self.settings_content().gap_3().whitespace_normal()
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
                        .flex_shrink_0()
                        .min_w_0()
                        .child(
                            div().flex_1().min_w_0().child("自动同步").child(
                                div().text_size(px(MIN_UI_FONT_SIZE)).child(
                                    "打开笔记库，以及新建、保存、删除或外部修改文件后自动同步。连续修改会稍等片刻再合并同步；关闭后仅在点击“立即同步”时同步。",
                                ),
                            ),
                        )
                        .child(
                            div().flex_shrink_0().child(super::settings_ui::setting_switch("webdav-auto")
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
                                        this.ui.cloud_sync.schedule.disable_automatic();
                                    }
                                    cx.notify();
                                }))),
                        ),
                )
                .child(div().flex().flex_wrap().flex_shrink_0().gap_2()
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
                    .child(Button::new("webdav-sync").primary().label(if state.schedule.busy { "正在处理……" } else { "立即同步" }).disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.request_cloud_sync(window, cx)))))
                .when(state.run.is_some(), |s| s.child(
                    Button::new("webdav-cancel-running").label("取消本次同步")
                        .disabled(state.run.as_ref().is_some_and(Run::is_cancelled))
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_running_sync(cx)))
                ))
                .when(!state.schedule.busy && (state.schedule.pending || state.schedule.again || state.schedule.wait.is_some() || state.watch.has_work()), |s| s.child(
                    Button::new("webdav-cancel-wait").label("取消本次同步等待")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_queued_sync();
                            this.cloud_message("已取消本次等待；开启自动同步时，后续变更或定期检查仍会触发同步。".into(), cx);
                        }))
                ))
                .child("双向同步笔记与附件，包含修改、重命名和删除。首次同步合并两端文件；同时修改时保留云端冲突副本，修改与删除冲突时保留修改。")
                .child("隐藏文件、空文件夹、工作区设置和历史记录不参与同步。单文件上限 128 MiB，一次下载上限 512 MiB。")
                .child(Button::new("webdav-poll-interval")
                    .label(format!("远端检查间隔：{} 分钟（点击切换）", self.ui.prefs.webdav.poll_minutes.clamp(1, 1440)))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.prefs.webdav.poll_minutes = match this.ui.prefs.webdav.poll_minutes {
                            1 => 5, 5 => 15, 15 => 30, _ => 1,
                        };
                        this.persist_workspace(cx);
                        cx.notify();
                    })))
                .child("自动同步开启时，应用持续打开会定期检查远端，重新激活窗口也会检查；未保存正文仍等待手动保存。")
                .when(state.run.is_some(), |s| {
                    let progress = state.progress.as_ref().filter(|p| p.total > 0);
                    s.child(gpui_component::progress::Progress::new("cloud-sync-progress")
                        .w_full()
                        .accessibility_label("当前同步阶段进度")
                        .loading(progress.is_none())
                        .value(progress.map_or(0., |p| p.completed as f32 * 100. / p.total as f32)))
                })
                .child(state.scheduling_label())
                .child(self.sync_success_label())
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
    fn background_work_waits_for_manual_note_save(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-manual-save-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("note.md"), "original").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("note.md".into(), Some("original".into()), false, window, cx);
                let editor = w.tabs[0].save.editor.clone();
                editor.update(cx, |state, cx| {
                    state.set_value("unsaved draft", window, cx);
                });
                w.flush_document_views(window, cx);
                assert!(w.tabs[0].save.persistence.is_dirty());
            })
            .unwrap();
        cx.run_until_parked();
        for _ in 0..3 {
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
            cx.run_until_parked();
        }
        handle
            .update(cx, |w, window, cx| {
                w.ui.cloud_sync.schedule.pending = true;
                w.ui.cloud_sync.schedule.automatic = true;
                w.tick_cloud_sync(window, cx);
                assert!(!w.tabs[0].save.persistence.is_saving());
                assert!(w.ui.cloud_sync.schedule.pending);
                w.ui.cloud_sync.schedule.pending = false;
                w.ui.cloud_sync.schedule.automatic = false;
                w.ui.backup.pending = Some(root.with_extension("backup"));
                w.tick_backups(window, cx);
                assert!(!w.tabs[0].save.persistence.is_saving());
                assert!(!w.ui.backup.busy);
                w.ui.backup.pending = None;
                assert!(w.tabs[0].save.persistence.is_dirty());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("note.md")).unwrap(),
            "original"
        );
        handle
            .update(cx, |w, window, cx| w.save_all(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("note.md")).unwrap(),
            "unsaved draft"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

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
                    w.ui.cloud_sync.schedule.pending = waiting;
                    w.ui.cloud_sync.schedule.automatic = waiting;
                    w.ui.cloud_sync.schedule.defer_quiet = waiting;
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
                assert!(!w.ui.cloud_sync.schedule.pending);
            })
            .unwrap();

        // Testing a corrected configuration must also bypass an automatic wait.
        let server = Server::new();
        handle
            .update(&mut visual, |w, window, cx| {
                configured(w, &server.url, true, window, cx);
                w.ui.cloud_sync.schedule.pending = true;
                w.ui.cloud_sync.schedule.automatic = true;
                w.ui.cloud_sync.schedule.defer_quiet = true;
                w.test_cloud_connection(cx);
                assert!(w.ui.cloud_sync.schedule.busy);
                assert_eq!(w.ui.prefs.webdav.url, server.url);
            })
            .unwrap();
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, _| {
                assert!(!w.ui.cloud_sync.schedule.busy);
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
                let run = Run::default();
                run.publish(Progress {
                    phase: Phase::Uploading,
                    completed: 3,
                    total: 8,
                    bytes: 2 * 1024 * 1024,
                });
                w.ui.cloud_sync.schedule.busy = true;
                w.ui.cloud_sync.run = Some(run.clone());
                w.poll_cloud_progress(cx);
                assert!(w.ui.cloud_sync.message.contains("3/8"));
                assert!(w.ui.cloud_sync.message.contains("37%"));
                assert!(w.ui.cloud_sync.message.contains("2.00 MiB"));
                assert_eq!(w.status, w.ui.cloud_sync.message);
                assert!(run.take_progress().is_none());
                assert_eq!(w.ui.cloud_sync.progress.as_ref().unwrap().completed, 3);
                w.apply_cloud_secret(None, window, cx);
                run.publish(Progress {
                    phase: Phase::Downloading,
                    completed: 1,
                    total: 2,
                    bytes: 128,
                });
                w.poll_cloud_progress(cx);
                assert!(w.ui.cloud_sync.message.is_empty());
                assert!(w.ui.cloud_sync.progress.is_none());
                assert!(w.ui.cloud_sync.run.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn replaced_run_rejects_old_completion(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-run-identity-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let handle = cx.add_window(Workspace::new);
        let replacement = Run::default();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.prefs.webdav.url = server.url.clone();
                w.ui.prefs.webdav.username = "user".into();
                configured(w, &server.url, true, window, cx);
                w.schedule_auto_sync(false);
                w.tick_cloud_sync(window, cx);
                let old = w.ui.cloud_sync.run.as_ref().unwrap();
                assert!(old.request_cancel());
                // Simulate a replacement in the same workspace generation: identity is
                // a separate guard from the vault-generation check.
                w.ui.cloud_sync.run = Some(replacement.clone());
                w.ui.cloud_sync.message = "replacement still active".into();
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.ui.cloud_sync.run.as_ref().unwrap().same(&replacement));
                assert!(w.ui.cloud_sync.schedule.busy && w.ui.file_operation);
                assert_eq!(w.ui.cloud_sync.message, "replacement still active");
                assert!(w.ui.cloud_sync.last_success.is_none());
                assert_eq!(
                    w.ui.pending_file_writes, 0,
                    "completed old worker releases only its write count"
                );
                // No second worker was launched by this fixture.
                w.ui.cloud_sync.run = None;
                w.ui.cloud_sync.schedule.finish();
                w.ui.file_operation = false;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    struct Server {
        url: String,
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        stop: Arc<AtomicBool>,
        offline: Arc<AtomicBool>,
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
            let offline = Arc::new(AtomicBool::new(false));
            let disconnected = offline.clone();
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
                    if disconnected.load(Ordering::Relaxed) {
                        continue;
                    }
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
                offline,
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
            ..Settings::default()
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
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.tick_cloud_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(w.ui.cloud_sync.message.contains("等待手动保存"));
                assert_eq!(
                    std::fs::read_to_string(root.join("local/note.md")).unwrap(),
                    "old"
                );
                w.save_all(window, cx);
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
                    !w.ui.cloud_sync.schedule.busy && !w.ui.cloud_sync.schedule.pending,
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(
                    w.ui.cloud_sync.message.starts_with("同步完成"),
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(w.ui.cloud_sync.progress.is_none());
                assert!(w.ui.cloud_sync.run.is_none());
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
                w.settings_save.test_error("disk full".into());
                w.ui.cloud_sync.schedule.pending = true;
                w.tick_cloud_sync(window, cx);
                assert!(!w.ui.cloud_sync.schedule.pending);
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
            ..Settings::default()
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
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(!w.ui.cloud_sync.schedule.pending);
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
            ..Settings::default()
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
                assert!(w.ui.cloud_sync.schedule.pending);
                assert!(w.ui.cloud_sync.schedule.wait.is_some());
                assert!(!w.ui.cloud_sync.schedule.busy);
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("revised", window, cx));
                w.tabs[0].save.persistence.test_set_dirty(true);
                w.save_pending(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
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
                assert!(
                    w.ui.cloud_sync.schedule.pending,
                    "quiet period should still hold"
                );
                assert!(w.ui.cloud_sync.schedule.wait.is_some());
                assert!(!w.ui.cloud_sync.schedule.busy);
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
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(!w.ui.cloud_sync.schedule.pending);
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
                    ..Settings::default()
                };
                configured(w, &server.url, true, window, cx);
                w.ui.discard_workspace_on_close = true;
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.wait.is_some());
                assert!(w.request_window_close(window, cx));
                assert!(w.ui.cloud_sync.schedule.busy);
                assert!(w.ui.cloud_sync.schedule.wait.is_none());
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
                w.ui.cloud_sync.schedule.busy = true;
                w.ui.pending_file_writes = 1;
                assert!(w.request_window_close(window, cx));
                w.ui.pending_file_writes = 2;
                assert!(!w.request_window_close(window, cx));
                w.ui.pending_file_writes = 1;
                w.add_tab("note.md".into(), Some("saved".into()), false, window, cx);
                w.tabs[0].save.persistence.test_set_conflict(true);
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
                    ..Settings::default()
                };
                w.apply_cloud_secret(Some(Ok(Some("secret".into()))), window, cx);
                assert!(!w.ui.cloud_sync.schedule.pending);
                w.schedule_auto_sync(true);
                w.tick(window, cx);
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn idle_device_receives_periodic_and_activation_updates_but_keeps_unsaved_text(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-remote-poll-{stamp}"));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let a = Vault::open(root.join("a"), root.join("recovery-a")).unwrap();
        let settings = Settings {
            url: server.url.clone(),
            username: "user".into(),
            ..Settings::default()
        };
        let remote = WebDav::new(&settings, "secret").unwrap();
        remote.prepare().unwrap();
        std::fs::write(a.root.join("note.md"), "version 1").unwrap();
        sync::synchronize(&a, &remote, &remote.identity()).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.join("b"), window, cx))
            .unwrap();
        cx.run_until_parked();
        let now = std::time::Instant::now();
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.webdav = settings.clone();
                configured(w, &server.url, true, window, cx);
                w.poll_remote_checks(now, false);
                assert!(!w.ui.cloud_sync.schedule.pending);
                w.poll_remote_checks(now + Duration::from_secs(300), false);
                assert!(w.ui.cloud_sync.schedule.pending);
                w.tick_cloud_sync(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("b/note.md")).unwrap(),
            "version 1"
        );
        std::fs::write(a.root.join("note.md"), "version 2").unwrap();
        sync::synchronize(&a, &remote, &remote.identity()).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.poll_remote_checks(now + Duration::from_secs(321), true);
                assert!(w.ui.cloud_sync.schedule.pending);
                w.tick_cloud_sync(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("b/note.md")).unwrap(),
            "version 2"
        );
        handle
            .update(cx, |w, window, cx| {
                w.open_note("note.md".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::write(a.root.join("note.md"), "version 3").unwrap();
        sync::synchronize(&a, &remote, &remote.identity()).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.tabs[w.active.unwrap()]
                    .save
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("unsaved B", window, cx));
                w.flush_document_views(window, cx);
                w.poll_remote_checks(now + Duration::from_secs(700), true);
                w.tick_cloud_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(w.tabs[w.active.unwrap()].save.persistence.is_dirty());
                w.watcher = None;
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("b/note.md")).unwrap(),
            "version 2"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn queued_sync_can_be_cancelled_or_left_by_switching_vault(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-sync-switch-{stamp}"));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.join("a"), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.webdav.url = "https://example.test/dav/".into();
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.wait.is_some());
                w.cancel_queued_sync();
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(w.ui.cloud_sync.schedule.wait.is_none());
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                w.add_tab("note.md".into(), Some("original".into()), false, window, cx);
                let editor = w.tabs[0].save.editor.clone();
                editor.update(cx, |s, cx| s.set_value("unsaved", window, cx));
                w.flush_document_views(window, cx);
                w.load_vault(root.join("b"), window, cx);
                assert_eq!(
                    w.vault.as_ref().unwrap().root,
                    root.join("a").canonicalize().unwrap()
                );
                assert!(
                    w.ui.cloud_sync.schedule.pending,
                    "rejected switch must preserve the queue"
                );
                editor.update(cx, |s, cx| s.set_value("original", window, cx));
                w.flush_document_views(window, cx);
                w.load_vault(root.join("b"), window, cx);
                assert!(w.loading);
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(w.ui.cloud_sync.schedule.wait.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(AUTO_SYNC_RETRY + AUTO_SYNC_QUIET);
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(
                    w.vault.as_ref().unwrap().root,
                    root.join("b").canonicalize().unwrap()
                );
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn external_binary_events_upload_and_completed_sync_events_do_not_loop(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-watch-upload-{stamp}"));
        std::fs::create_dir_all(root.join("local")).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("local"), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let (sender, receiver) = std::sync::mpsc::channel();
        let path = root
            .join("local")
            .canonicalize()
            .unwrap()
            .join("external.bin");
        std::fs::write(&path, [0, 9, 255]).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.watcher = None;
                w.watch_events = Some(receiver);
                w.ui.prefs.webdav = Settings {
                    url: server.url.clone(),
                    username: "user".into(),
                    ..Settings::default()
                };
                configured(w, &server.url, true, window, cx);
                sender
                    .send(Ok(notify::Event::new(notify::EventKind::Modify(
                        notify::event::ModifyKind::Any,
                    ))
                    .add_path(path.clone())))
                    .unwrap();
                w.tick(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.tick_sync_watch_at(std::time::Instant::now() + Duration::from_secs(6), cx)
            })
            .unwrap();
        cx.run_until_parked();
        for _ in 0..4 {
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
            cx.run_until_parked();
        }
        assert!(
            server
                .files
                .lock()
                .unwrap()
                .values()
                .any(|bytes| bytes == &[0, 9, 255])
        );
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.busy);
                sender
                    .send(Ok(notify::Event::new(notify::EventKind::Modify(
                        notify::event::ModifyKind::Any,
                    ))
                    .add_path(path)))
                    .unwrap();
                w.tick(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.tick_sync_watch_at(std::time::Instant::now() + Duration::from_secs(6), cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(
                    !w.ui.cloud_sync.schedule.pending,
                    "unchanged sync output must not queue a new transfer"
                );
                assert!(!w.ui.cloud_sync.schedule.again);
            })
            .unwrap();
        let completed = handle
            .update(cx, |w, _, cx| {
                let completed =
                    w.ui.cloud_sync
                        .last_success
                        .expect("successful HTTP sync records time");
                assert!(!w.sync_success_label().contains("暂无"));
                w.load_sync_success(cx);
                assert_eq!(w.ui.cloud_sync.last_success, None);
                completed
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.ui.cloud_sync.last_success, Some(completed));
                // A result for the old account must not appear under the next account.
                w.load_sync_success(cx);
                w.ui.prefs.webdav.username = "another-account".into();
                w.load_sync_success(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.ui.cloud_sync.last_success, None);
                assert!(w.sync_success_label().contains("暂无时间记录"));
            })
            .unwrap();
        // Cancellation invalidates a classification already running in the background.
        std::fs::write(root.join("local/external.bin"), [1, 2, 3]).unwrap();
        handle
            .update(cx, |w, _, cx| {
                w.note_sync_watch_paths(vec![PathBuf::from("external.bin")]);
                w.tick_sync_watch_at(std::time::Instant::now() + Duration::from_secs(6), cx);
                w.cancel_queued_sync();
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(!w.ui.cloud_sync.schedule.pending);
                assert!(!w.ui.cloud_sync.schedule.again);
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn offline_retry_preserves_backoff_and_converges_after_reconnection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        server.offline.store(true, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "inkstone-offline-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("local")).unwrap();
        std::fs::write(root.join("local/note.md"), "before").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("local"), root.join("recovery")).unwrap());
                w.ui.prefs.webdav = Settings {
                    url: server.url.clone(),
                    username: "user".into(),
                    ..Settings::default()
                };
                configured(w, &server.url, true, window, cx);
                w.schedule_auto_sync(false);
                w.tick_cloud_sync(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(w.ui.cloud_sync.message.starts_with("同步未完成"));
                assert!(w.ui.cloud_sync.last_success.is_none());
                assert!(w.ui.cloud_sync.scheduling_label().contains("重试"));
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.wait.is_some());
                w.schedule_auto_sync(true);
                w.pump_auto_sync(window, cx);
                assert!(w.ui.cloud_sync.schedule.retry_waiting());
                assert!(!w.ui.cloud_sync.schedule.defer_quiet);
            })
            .unwrap();
        std::fs::write(root.join("local/note.md"), "latest while offline").unwrap();
        server.offline.store(false, Ordering::Relaxed);
        cx.executor().advance_clock(AUTO_SYNC_QUIET);
        cx.run_until_parked();
        assert!(
            server.files.lock().unwrap().is_empty(),
            "local edits must not shorten retry backoff"
        );
        cx.executor()
            .advance_clock(AUTO_SYNC_RETRY - AUTO_SYNC_QUIET);
        cx.run_until_parked();
        for _ in 0..4 {
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
            cx.run_until_parked();
        }
        handle
            .update(cx, |w, _, _| {
                assert!(
                    w.ui.cloud_sync.message.starts_with("同步完成"),
                    "{}",
                    w.ui.cloud_sync.message
                );
                assert!(w.ui.cloud_sync.last_success.is_some());
                assert_eq!(w.ui.cloud_sync.scheduling_label(), "当前没有同步任务");
            })
            .unwrap();
        assert!(
            server
                .files
                .lock()
                .unwrap()
                .values()
                .any(|bytes| bytes == b"latest while offline")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn cancelling_running_sync_waits_for_worker_and_does_not_retry(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let server = Server::new();
        let root = std::env::temp_dir().join(format!(
            "inkstone-cancel-ui-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("local")).unwrap();
        std::fs::write(root.join("local/note.md"), "local").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("local"), root.join("recovery")).unwrap());
                w.ui.prefs.webdav = Settings {
                    url: server.url.clone(),
                    username: "user".into(),
                    ..Settings::default()
                };
                configured(w, &server.url, true, window, cx);
                w.schedule_auto_sync(false);
                w.tick_cloud_sync(window, cx);
                w.cancel_running_sync(cx);
                assert!(w.ui.cloud_sync.schedule.busy);
                assert!(w.ui.file_operation);
                assert!(w.ui.pending_file_writes > 0);
                assert!(w.ui.cloud_sync.scheduling_label().contains("正在取消"));
                assert!(!w.ui.cloud_sync.schedule.pending);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.ui.cloud_sync.schedule.busy);
                assert!(!w.ui.file_operation);
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(w.ui.cloud_sync.message.starts_with("本次同步已取消"));
                assert!(w.ui.cloud_sync.run.is_none());
                assert!(w.ui.cloud_sync.last_success.is_none());
                w.pump_auto_sync(window, cx);
                assert!(!w.ui.cloud_sync.schedule.retry_waiting());
            })
            .unwrap();
        cx.executor().advance_clock(AUTO_SYNC_RETRY);
        cx.run_until_parked();
        assert!(
            !server
                .files
                .lock()
                .unwrap()
                .keys()
                .any(|p| p.ends_with("manifest.json"))
        );
        assert_eq!(
            std::fs::read_to_string(root.join("local/note.md")).unwrap(),
            "local"
        );
        handle
            .update(cx, |w, window, cx| w.request_cloud_sync(window, cx))
            .unwrap();
        for _ in 0..4 {
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
        }
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.ui.cloud_sync.last_success.is_some());
                assert!(!w.ui.cloud_sync.schedule.busy);
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
}
