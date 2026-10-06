//! Cloud cleanup owns a separate request lifetime; its execution ticket remains
//! held until the worker finishes, including after cancellation or form changes.
use super::super::focus_reveal::FocusReveal;
use super::capacity::Key;
use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    request: u64,
    key: Option<Key>,
    loading: bool,
    preview: Option<sync::CloudCleanupPreview>,
    confirmation: bool,
    run: Option<CleanupRun>,
    message: String,
}
struct CleanupRun {
    request: u64,
    key: Key,
    cancellation: Arc<sync::Cancellation>,
}
impl State {
    pub(super) fn is_running(&self) -> bool {
        self.run.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.key = None;
        self.loading = false;
        self.preview = None;
        self.confirmation = false;
        self.message.clear();
        if let Some(run) = &self.run {
            run.cancellation.request();
        }
    }
    fn finish_preview(
        &mut self,
        request: u64,
        key: Key,
        current: Key,
        result: Result<sync::CloudCleanupPreview, String>,
    ) {
        if self.request != request || self.key != Some(key) {
            return;
        }
        self.loading = false;
        if key != current {
            self.invalidate();
            return;
        }
        match result {
            Ok(preview) => {
                self.message = if preview.candidates == 0 {
                    "没有可清理的云端旧对象。".into()
                } else {
                    "预览完成；执行前还会重新核对并校验全部云端对象。".into()
                };
                self.preview = Some(preview);
            }
            Err(error) => self.message = format!("云端清理预览失败：{error}"),
        }
    }
    fn finish_run(&mut self, request: u64, key: Key, current: Key, message: String) -> bool {
        if !self
            .run
            .as_ref()
            .is_some_and(|run| run.request == request && run.key == key)
        {
            return false;
        }
        self.run = None;
        if self.key != Some(key) || current != key {
            return false;
        }
        self.message = message;
        true
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(run) = &self.run {
            run.cancellation.request();
        }
    }
}

impl Workspace {
    fn refresh_cloud_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.cloud_settings_disabled() || self.ui.cloud_sync.cleanup.loading {
            return;
        }
        let key = self.cloud_capacity_key(cx);
        let state = &mut self.ui.cloud_sync.cleanup;
        state.invalidate();
        let request = state.request;
        state.key = Some(key);
        state.loading = true;
        state.message = "正在读取云端清理预览……".into();
        let settings = self.cloud_settings(cx);
        if let Err(error) = settings.validate() {
            self.ui
                .cloud_sync
                .cleanup
                .finish_preview(request, key, key, Err(error.to_string()));
            cx.notify();
            return;
        }
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        let task = cx.background_executor().spawn(async move {
            WebDav::new(&settings, &password)
                .and_then(|remote| remote.preview_cloud_cleanup())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let current = this.cloud_capacity_key(cx);
                this.ui
                    .cloud_sync
                    .cleanup
                    .finish_preview(request, key, current, result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn can_clean_cloud(&self, cx: &App) -> bool {
        let state = &self.ui.cloud_sync.cleanup;
        !self.cloud_settings_disabled()
            && !state.loading
            && state.key == Some(self.cloud_capacity_key(cx))
            && self.file_writes.can_start_exclusive_operation()
            && state
                .preview
                .as_ref()
                .is_some_and(|preview| preview.candidates > 0)
    }
    fn request_cloud_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.can_clean_cloud(cx) {
            self.ui.cloud_sync.cleanup.confirmation = true;
            cx.notify();
        }
    }
    fn execute_cloud_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.can_clean_cloud(cx) || !self.ui.cloud_sync.cleanup.confirmation {
            return;
        }
        // Admission was checked above on the UI thread. This task only mutates
        // remote objects: coordinate sync/switching without blocking local saves.
        let ticket = self.file_writes.begin();
        let settings = self.cloud_settings(cx);
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        let key = self.cloud_capacity_key(cx);
        let state = &mut self.ui.cloud_sync.cleanup;
        let request = state.request;
        let preview = state.preview.take().unwrap();
        let cancellation = Arc::new(sync::Cancellation::default());
        state.run = Some(CleanupRun {
            request,
            key,
            cancellation: cancellation.clone(),
        });
        state.confirmation = false;
        state.message = "正在校验并清理云端旧对象；请等待当前步骤完成……".into();
        self.ui.cloud_sync.capacity.invalidate();
        let task = cx.background_executor().spawn(async move {
            let result = (|| {
                let remote = WebDav::new(&settings, &password)?;
                remote
                    .prepare_cloud_cleanup(&preview, &cancellation)?
                    .execute(&cancellation)
            })();
            match result {
                Ok(report) => format!(
                    "已确认清理 {} 个云端旧对象，共 {} 字节。可重新预览或刷新容量。",
                    report.removed, report.bytes
                ),
                Err(error) if sync::is_cloud_cleanup_cancelled(&error) => {
                    format!("已停止云端清理：{error}。请重新预览后检查剩余对象。")
                }
                Err(error) => format!(
                    "云端清理未完成：{error}。请重新预览后检查；已完成的格式升级及删除不会撤销。"
                ),
            }
        });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(ticket);
                let current = this.cloud_capacity_key(cx);
                if this
                    .ui
                    .cloud_sync
                    .cleanup
                    .finish_run(request, key, current, message.clone())
                {
                    this.notifications.publish(message);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn stop_cloud_cleanup(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.ui.cloud_sync.cleanup.run {
            run.cancellation.request();
            self.ui.cloud_sync.cleanup.message =
                "已请求停止，正在等待当前请求结束；已完成的删除不会撤销。".into();
            cx.notify();
        }
    }
    pub(super) fn cloud_cleanup_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.cloud_sync.cleanup;
        let current = state.key == Some(self.cloud_capacity_key(cx));
        div().flex().flex_col().gap_2().min_w_0().whitespace_normal()
            .child("云端旧对象清理")
            .child("只读预览按当前连接表单读取，不保存配置或笔记。清理需服务器支持独占维护，期间其他设备的同步可能暂时失败，稍后可重试。")
            .child(FocusReveal::new("cloud-cleanup-preview-focus", &self.ui.settings_scroll,
                div().debug_selector(|| "cloud-cleanup-preview-control".into()).child(Button::new("cloud-cleanup-preview").label("预览云端清理")
                    .disabled(self.cloud_settings_disabled() || state.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_cloud_cleanup(cx))))))
            .when(current && !state.message.is_empty(), |s| s.child(state.message.clone()))
            .when_some(state.preview.as_ref().filter(|_| current), |s, preview| s
                .child(format!("预计清理：{} 个旧对象 · {:.2} MiB；保留当前引用：{} 个对象 · {:.2} MiB", preview.candidates, preview.candidate_bytes as f64 / 1048576., preview.retained, preview.retained_bytes as f64 / 1048576.))
                .when(preview.candidates > 0 && !state.confirmation, |s| s.child(FocusReveal::new("cloud-cleanup-review-focus", &self.ui.settings_scroll,
                    div().debug_selector(|| "cloud-cleanup-review-control".into()).child(Button::new("cloud-cleanup-review").label("查看清理确认")
                        .disabled(!self.can_clean_cloud(cx))
                        .on_click(cx.listener(|this, _, window, cx| { this.request_cloud_cleanup(cx); window.focus(&this.ui.modal_focus, cx); })))))))
            .when(current && state.confirmation, |s| s
                .child("请先更新所有设备的墨砚。首次清理会升级云端格式，旧版本将无法继续同步；升级及删除不能撤销。本地笔记与恢复记录保留。")
                .child(FocusReveal::new("cloud-cleanup-confirm-focus", &self.ui.settings_scroll,
                    div().debug_selector(|| "cloud-cleanup-confirm-control".into()).child(Button::new("cloud-cleanup-confirm").label("确认清理").danger()
                        .disabled(!self.can_clean_cloud(cx))
                        .on_click(cx.listener(|this, _, window, cx| { this.execute_cloud_cleanup(cx); window.focus(&this.ui.modal_focus, cx); })))))
                .child(FocusReveal::new("cloud-cleanup-cancel-focus", &self.ui.settings_scroll,
                    div().debug_selector(|| "cloud-cleanup-cancel-control".into()).child(Button::new("cloud-cleanup-cancel").label("取消")
                        .on_click(cx.listener(|this, _, window, cx| { this.ui.cloud_sync.cleanup.confirmation = false; window.focus(&this.ui.modal_focus, cx); cx.notify(); }))))))
            .when_some(state.run.as_ref(), |s, run| s
                .when(!current, |s| s.child("原连接的云端清理正在停止，请等待当前请求结束。"))
                .child(FocusReveal::new("cloud-cleanup-stop-focus", &self.ui.settings_scroll,
                    div().debug_selector(|| "cloud-cleanup-stop-control".into()).child(Button::new("cloud-cleanup-stop").label(if run.cancellation.is_requested() { "正在停止" } else { "停止后续清理" })
                        .disabled(run.cancellation.is_requested())
                        .on_click(cx.listener(|this, _, window, cx| { this.stop_cloud_cleanup(cx); window.focus(&this.ui.modal_focus, cx); }))))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
