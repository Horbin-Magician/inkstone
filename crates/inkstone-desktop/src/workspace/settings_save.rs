//! Settings persistence and its independently testable completion state.
use super::*;

#[derive(Default)]
pub(super) struct State {
    last: String,
    error: Option<String>,
    active: Option<u64>,
    revision: u64,
}
impl State {
    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }
    pub fn error(&self) -> &Option<String> {
        &self.error
    }
    pub fn last(&self) -> &str {
        &self.last
    }
    pub fn retry(&mut self) {
        self.error = None;
    }
    pub fn reset(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.active = None;
        self.error = None;
        self.last.clear();
    }
    pub fn begin(&mut self, serialized: &str) -> Option<u64> {
        if self.is_busy() || self.error.is_some() || serialized == self.last {
            return None;
        }
        self.revision = self.revision.wrapping_add(1);
        self.active = Some(self.revision);
        self.active
    }
    pub fn abandon(&mut self, ticket: u64) {
        if self.active == Some(ticket) {
            self.active = None;
        }
    }
    pub fn complete(&mut self, ticket: u64, result: Result<String, String>) -> bool {
        if self.active != Some(ticket) {
            return false;
        }
        self.active = None;
        match result {
            Ok(serialized) => {
                self.last = serialized;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
    #[cfg(test)]
    pub fn test_error(&mut self, error: String) {
        self.error = Some(error);
    }
}

impl Workspace {
    pub(super) fn persist_workspace(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.ui.backup.busy {
            return;
        }
        self.snapshot_views(cx);
        let Some(vault) = &self.vault else {
            return;
        };
        if self.loading
            || self.settings_save.is_busy()
            || self.settings_save.error().is_some()
            || self.ui.discard_workspace_on_close
        {
            return;
        }
        self.ui.prefs.open_paths = self.tabs.iter().map(|t| t.path.clone()).collect();
        self.ui.prefs.left_panel = self.ui.left_mode;
        self.ui.prefs.right_panel = self.ui.right_mode;
        self.ui.prefs.active_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        self.ui.prefs.active_tab_index = self.active;
        let Ok(serialized) = serde_json::to_string(&self.ui.prefs) else {
            return;
        };
        let Some(ticket) = self.settings_save.begin(&serialized) else {
            return;
        };
        let prefs = self.ui.prefs.clone();
        let path = vault.root.join(".inkstone-workspace.json");
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { prefs.save(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    this.settings_save.abandon(ticket);
                    return;
                }
                if !this.settings_save.complete(
                    ticket,
                    result.map(|()| serialized).map_err(|e| e.to_string()),
                ) {
                    return;
                }
                if let Some(error) = this.settings_save.error() {
                    this.ui.window_close_requested = false;
                    this.notifications.publish(format!(
                        "无法保存工作区设置：{error}。可重试或不保存布局并关闭。"
                    ));
                    cx.notify();
                } else {
                    this.persist_workspace(cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::State;
    #[test]
    fn coalesces_changes_and_requires_explicit_retry_after_failure() {
        let mut state = State::default();
        let first = state.begin("first").unwrap();
        assert!(state.begin("latest").is_none());
        assert!(state.complete(first, Ok("first".into())));
        assert!(state.begin("first").is_none());
        let latest = state.begin("latest").unwrap();
        assert!(state.complete(latest, Err("disk full".into())));
        assert_eq!(state.last(), "first");
        assert!(state.begin("latest").is_none());
        state.retry();
        let retry = state.begin("latest").unwrap();
        assert!(state.complete(retry, Ok("latest".into())));
        assert!(!state.is_busy());
        assert_eq!(state.last(), "latest");
        assert!(state.error().is_none());
    }
    #[test]
    fn old_vault_completions_cannot_release_or_poison_new_save() {
        let mut state = State::default();
        let old = state.begin("old vault").unwrap();
        state.reset();
        let current = state.begin("new vault").unwrap();
        state.abandon(old);
        assert!(!state.complete(old, Err("old error".into())));
        assert!(state.is_busy());
        assert!(state.error().is_none());
        assert!(state.complete(current, Ok("new vault".into())));
        assert!(!state.complete(old, Ok("old vault".into())));
        assert_eq!(state.last(), "new vault");
        let pending = state.begin("new edits").unwrap();
        // Failed navigation may leave the original state installed: release only its old job.
        state.abandon(pending);
        assert!(!state.is_busy());
        assert!(state.begin("new edits").is_some());
    }
}
