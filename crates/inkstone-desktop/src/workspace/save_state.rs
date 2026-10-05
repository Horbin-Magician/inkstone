//! Document persistence transitions, independent of editor entities and windows.
use std::cell::{Cell, RefCell};

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
}
