//! Read-only capacity work has its own lifetime; it must not hold up saving or sync.
use super::*;
use sha2::{Digest, Sha256};

type Key = (u64, [u8; 32]);
#[derive(Default)]
pub(super) struct State {
    request: u64,
    key: Option<Key>,
    busy: bool,
    report: Option<sync::CloudCapacity>,
    message: String,
}
impl State {
    pub(super) fn invalidate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.key = None;
        self.busy = false;
        self.report = None;
        self.message.clear();
    }
    fn begin(&mut self, key: Key) -> u64 {
        self.request = self.request.wrapping_add(1);
        self.key = Some(key);
        self.busy = true;
        self.report = None;
        self.message = "正在读取云端对象容量……".into();
        self.request
    }
    fn finish(
        &mut self,
        request: u64,
        key: Key,
        current: Key,
        result: Result<sync::CloudCapacity, String>,
    ) {
        if request != self.request || self.key != Some(key) {
            return;
        }
        self.busy = false;
        if key != current {
            self.report = None;
            self.message.clear();
            return;
        }
        match result {
            Ok(report) => {
                self.report = Some(report);
                let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                self.message = format!("容量统计时间：{time}；后续变化请重新刷新。");
            }
            Err(error) => self.message = format!("读取云端容量失败：{error}"),
        }
    }
}
impl Workspace {
    fn cloud_capacity_key(&self, cx: &App) -> Key {
        let settings = self.cloud_settings(cx);
        // Retain no extra plaintext password in the capacity state or messages.
        let value = serde_json::to_vec(&(
            settings.url,
            settings.username,
            self.ui.cloud_sync.password.read(cx).value(),
        ))
        .unwrap();
        (self.generation, Sha256::digest(value).into())
    }
    fn refresh_cloud_capacity(&mut self, cx: &mut Context<Self>) {
        let key = self.cloud_capacity_key(cx);
        let state = &self.ui.cloud_sync.capacity;
        if self.cloud_settings_disabled() || state.busy && state.key == Some(key) {
            return;
        }
        let request = self.ui.cloud_sync.capacity.begin(key);
        let settings = self.cloud_settings(cx);
        if let Err(error) = settings.validate() {
            self.ui
                .cloud_sync
                .capacity
                .finish(request, key, key, Err(error.to_string()));
            cx.notify();
            return;
        }
        let password = self.ui.cloud_sync.password.read(cx).value().to_string();
        // Use the current form without persisting its settings, password or notes.
        let task = cx.background_executor().spawn(async move {
            WebDav::new(&settings, &password)
                .and_then(|remote| remote.capacity())
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let current = this.cloud_capacity_key(cx);
                this.ui
                    .cloud_sync
                    .capacity
                    .finish(request, key, current, result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn cloud_capacity_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.cloud_sync.capacity;
        let current = state.key == Some(self.cloud_capacity_key(cx));
        div().flex().flex_col().gap_2().min_w_0().whitespace_normal()
            .child("云端对象容量")
            .child(super::super::settings_ui::SettingsFocusTarget::new("cloud-capacity-focus", &self.ui.settings_scroll,
                div().debug_selector(|| "cloud-capacity-control".into()).child(Button::new("cloud-capacity-refresh").label("刷新云端容量")
                    .disabled(self.cloud_settings_disabled() || current && state.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_cloud_capacity(cx))))))
            .child("按当前表单读取云端，不保存配置或笔记。仅统计服务器报告的对象长度，不含清单及服务器存储开销；不代表内容已校验。")
            .when(current && !state.message.is_empty(), |s| s.child(state.message.clone()))
            .when_some(state.report.as_ref().filter(|_| current), |s, report| s
                .child(format!("当前清单引用：{} 个对象 · {:.2} MiB", report.referenced_objects, report.referenced_bytes as f64 / 1048576.))
                .child(format!("当前清单未引用：{} 个对象 · {:.2} MiB", report.unreferenced_objects, report.unreferenced_bytes as f64 / 1048576.))
                .child(format!("缺失或大小未知的引用：{} 个；无法识别的列表项：{} 条", report.missing_or_unknown_objects, report.unrecognized_entries)))
            .child("未引用对象可能属于其他设备尚未完成的同步，不等于可释放空间。这里只提供统计，不提供云端清理。")
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[test]
    fn stale_capacity_results_cannot_replace_new_requests_or_accounts() {
        let mut state = State::default();
        let first = (1, [1; 32]);
        let other_account = (1, [2; 32]);
        let other_vault = (2, [1; 32]);
        let old = state.begin(first);
        let new = state.begin(other_account);
        state.finish(old, first, other_account, Ok(Default::default()));
        assert!(state.busy && state.report.is_none());
        state.finish(new, other_account, other_vault, Ok(Default::default()));
        assert!(!state.busy && state.report.is_none() && state.message.is_empty());
        let new = state.begin(other_vault);
        state.finish(new, other_vault, other_vault, Err("offline".into()));
        assert!(!state.busy && state.report.is_none() && state.message.contains("offline"));
        let retry = state.begin(other_vault);
        state.finish(
            retry,
            other_vault,
            other_vault,
            Ok(sync::CloudCapacity {
                unreferenced_bytes: 42,
                ..Default::default()
            }),
        );
        assert_eq!(state.report.unwrap().unreferenced_bytes, 42);
    }
    #[gpui::test]
    fn capacity_form_validation_does_not_persist_or_queue_sync(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-capacity-form-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/note.md"), "original").unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.cloud_sync
                    .url
                    .update(cx, |s, cx| s.set_value("invalid URL", window, cx));
                let first = w.cloud_capacity_key(cx);
                w.ui.cloud_sync
                    .username
                    .update(cx, |s, cx| s.set_value("another user", window, cx));
                let username = w.cloud_capacity_key(cx);
                assert_ne!(first, username);
                w.ui.cloud_sync
                    .password
                    .update(cx, |s, cx| s.set_value("unsaved password", window, cx));
                let password = w.cloud_capacity_key(cx);
                assert_ne!(username, password);
                w.generation += 1;
                assert_ne!(password, w.cloud_capacity_key(cx));
                w.refresh_cloud_capacity(cx);
                assert!(!w.ui.cloud_sync.capacity.busy);
                assert!(w.ui.cloud_sync.capacity.message.contains("失败"));
                assert!(w.ui.prefs.webdav.url.is_empty());
                assert!(w.ui.prefs.webdav.username.is_empty());
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(!w.ui.cloud_sync.is_busy() && !w.ui.cloud_sync.is_pending());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("vault/note.md")).unwrap(),
            "original"
        );
        assert!(!root.join("vault/.inkstone-workspace.json").exists());
        let (request, old_key) = handle
            .update(cx, |w, window, cx| {
                let key = w.cloud_capacity_key(cx);
                let request = w.ui.cloud_sync.capacity.begin(key);
                w.ui.cloud_sync.password.update(cx, |s, cx| {
                    s.set_value("changed during request", window, cx);
                    cx.emit(InputEvent::Change);
                });
                (request, key)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.ui.cloud_sync.capacity.busy);
                assert!(w.ui.cloud_sync.capacity.key.is_none());
                let current = w.cloud_capacity_key(cx);
                w.ui.cloud_sync
                    .capacity
                    .finish(request, old_key, current, Ok(Default::default()));
                assert!(w.ui.cloud_sync.capacity.report.is_none());
            })
            .unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }
}
