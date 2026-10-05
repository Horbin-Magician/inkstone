//! Document persistence transitions, independent of editor entities and windows.
use std::cell::{Cell, RefCell};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ExternalChange {
    Unchanged,
    PreserveLocal,
    Reload(String),
}

pub(super) struct SaveState {
    pub baseline: RefCell<Option<String>>,
    pub dirty: Cell<bool>,
    pub saving: Cell<bool>,
    pub conflict: Cell<bool>,
    pub error: RefCell<Option<String>>,
    pub recovery_text: RefCell<String>,
}
impl SaveState {
    pub fn new(baseline: Option<String>, dirty: bool) -> Self {
        Self {
            baseline: RefCell::new(baseline),
            dirty: Cell::new(dirty),
            saving: Cell::new(false),
            conflict: Cell::new(false),
            error: RefCell::new(None),
            recovery_text: RefCell::new(String::new()),
        }
    }
    pub fn edited(&self, current: &str) {
        self.dirty
            .set(self.baseline.borrow().as_deref() != Some(current));
    }
    pub fn retry(&self) {
        if !self.conflict.get() {
            self.error.replace(None);
        }
    }
    pub fn begin(&self) -> bool {
        if !self.dirty.get()
            || self.saving.get()
            || self.conflict.get()
            || self.error.borrow().is_some()
        {
            return false;
        }
        self.saving.set(true);
        true
    }
    pub fn saved(&self, text: String, current: &str) {
        self.saving.set(false);
        self.error.replace(None);
        self.baseline.replace(Some(text));
        self.edited(current);
    }
    pub fn failed(&self, conflict: bool, error: String) {
        self.saving.set(false);
        self.conflict.set(conflict);
        self.error.replace(Some(error));
    }
    /// Called only after the caller validates the comparison snapshot and pending input.
    pub fn begin_resolution(&self) -> bool {
        if self.saving.replace(true) {
            return false;
        }
        true
    }
    pub fn finish_operation(&self) {
        self.saving.set(false);
    }
    pub fn resolved(&self, text: String, newer_input_error: Option<String>) {
        self.baseline.replace(Some(text));
        self.conflict.set(newer_input_error.is_some());
        if newer_input_error.is_some() {
            self.dirty.set(true);
        }
        self.error.replace(newer_input_error);
        // The caller installs the chosen text only if no input arrived, then calls edited.
    }
    /// Rename/trash require a saved document, but do not write its current contents.
    pub fn begin_file_operation(&self) -> bool {
        if self.dirty.get()
            || self.saving.get()
            || self.conflict.get()
            || self.baseline.borrow().is_none()
        {
            return false;
        }
        self.saving.set(true);
        true
    }
    /// Apply a completed batch/link edit only while its reviewed snapshot still matches.
    pub fn apply_reviewed_edit(
        &self,
        before: &str,
        after: &str,
        current: &str,
        pending_input: bool,
    ) -> bool {
        if pending_input || current != before || self.baseline.borrow().as_deref() != Some(before) {
            self.preserve_external_change();
            return false;
        }
        self.baseline.replace(Some(after.to_string()));
        true
    }
    pub fn preserve_external_change(&self) {
        self.conflict.set(true);
        self.dirty.set(true);
    }
    /// The caller rejects stale snapshots / active saves before applying disk results.
    pub fn external_change(&self, disk: Option<String>, pending_input: bool) -> ExternalChange {
        if disk == *self.baseline.borrow() {
            return ExternalChange::Unchanged;
        }
        if self.dirty.get() || pending_input || disk.is_none() {
            self.preserve_external_change();
            return ExternalChange::PreserveLocal;
        }
        let text = disk.expect("missing files preserve local contents");
        self.baseline.replace(Some(text.clone()));
        ExternalChange::Reload(text)
    }
    pub fn prepare_copy(&self) {
        self.baseline.replace(None);
        self.conflict.set(false);
        self.error.replace(None);
        self.dirty.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_during_save_stay_dirty_and_retry_never_bypasses_conflict() {
        let state = SaveState::new(Some("disk".into()), false);
        assert!(!state.begin());
        state.edited("first");
        assert!(state.begin());
        assert!(!state.begin(), "shared views cannot start duplicate saves");
        state.saved("first".into(), "newer input");
        assert!(state.dirty.get());
        assert_eq!(state.baseline.borrow().as_deref(), Some("first"));
        assert!(state.begin());
        state.failed(false, "disk full".into());
        assert!(!state.begin());
        state.retry();
        assert!(state.begin());
        state.failed(true, "external change".into());
        state.retry();
        assert!(!state.begin());
        assert!(state.error.borrow().is_some());
        state.prepare_copy();
        assert!(state.baseline.borrow().is_none());
        assert!(state.begin());
        state.saved("newer input".into(), "newer input");
        assert!(!state.dirty.get() && !state.saving.get() && !state.conflict.get());
    }
    #[test]
    fn external_changes_preserve_unsaved_pending_missing_and_unreadable_contents() {
        for (dirty, pending, disk) in [
            (true, false, Some("external".to_string())),
            (false, true, Some("external".to_string())),
            (false, false, None),
        ] {
            let state = SaveState::new(Some("original".into()), dirty);
            assert_eq!(
                state.external_change(disk, pending),
                ExternalChange::PreserveLocal
            );
            assert_eq!(state.baseline.borrow().as_deref(), Some("original"));
            assert!(state.dirty.get() && state.conflict.get());
            state.retry();
            assert!(!state.begin());
        }
        let state = SaveState::new(Some("original".into()), false);
        assert_eq!(
            state.external_change(Some("original".into()), true),
            ExternalChange::Unchanged
        );
        assert!(!state.dirty.get() && !state.conflict.get());
        assert_eq!(
            state.external_change(Some("external".into()), false),
            ExternalChange::Reload("external".into())
        );
        state.edited("external");
        assert!(!state.dirty.get());
        state.preserve_external_change(); // A read error must preserve the last known baseline.
        assert_eq!(state.baseline.borrow().as_deref(), Some("external"));
        assert!(state.dirty.get() && state.conflict.get());
    }

    #[test]
    fn resolution_keeps_new_input_protected_until_reviewed_again() {
        let state = SaveState::new(Some("original".into()), true);
        state.preserve_external_change();
        assert!(state.begin_resolution());
        assert!(!state.begin_resolution());
        state.finish_operation();
        state.resolved("chosen disk".into(), Some("new input arrived".into()));
        assert_eq!(state.baseline.borrow().as_deref(), Some("chosen disk"));
        assert!(state.dirty.get() && state.conflict.get());
        state.retry();
        assert!(!state.begin());
        assert!(state.begin_resolution());
        state.finish_operation();
        state.failed(true, "disk changed again".into());
        assert_eq!(state.baseline.borrow().as_deref(), Some("chosen disk"));
        assert!(state.conflict.get() && !state.saving.get());
        assert!(state.begin_resolution());
        state.finish_operation();
        state.resolved("reviewed".into(), None);
        state.edited("reviewed");
        assert!(!state.dirty.get() && !state.conflict.get() && !state.saving.get());
        assert!(state.error.borrow().is_none());
    }
    #[test]
    fn reviewed_edits_reject_new_input_and_stale_baselines() {
        for (baseline, current, pending) in [
            ("before", "new input", false),
            ("before", "before", true),
            ("changed baseline", "before", false),
        ] {
            let state = SaveState::new(Some(baseline.into()), false);
            assert!(!state.apply_reviewed_edit("before", "after", current, pending));
            assert_eq!(state.baseline.borrow().as_deref(), Some(baseline));
            assert!(state.dirty.get() && state.conflict.get());
            assert!(!state.begin());
        }
        let state = SaveState::new(Some("before".into()), false);
        assert!(state.apply_reviewed_edit("before", "after", "before", false));
        state.edited("after");
        assert_eq!(state.baseline.borrow().as_deref(), Some("after"));
        assert!(!state.dirty.get() && !state.conflict.get());
    }

    #[test]
    fn file_operations_require_saved_contents_and_keep_edits_during_operation() {
        let state = SaveState::new(None, true);
        assert!(!state.begin_file_operation());
        state.saved("disk".into(), "disk");
        assert!(state.begin_file_operation());
        assert!(!state.begin_file_operation());
        state.edited("typed during rename");
        state.finish_operation();
        assert!(state.dirty.get() && !state.saving.get());
        assert_eq!(state.baseline.borrow().as_deref(), Some("disk"));
        assert!(!state.begin_file_operation());
        state.preserve_external_change(); // Trash completed while local edits arrived.
        assert!(state.conflict.get());
        state.retry();
        assert!(!state.begin());
    }
}
