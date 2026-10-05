//! Identity and cross-thread signals for one transfer; independent of its window.
use inkstone_core::vault::sync::{Cancellation, Progress};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(super) struct Run(Arc<Signals>);
#[derive(Default)]
struct Signals {
    cancellation: Cancellation,
    progress: Mutex<Option<Progress>>,
}
impl Run {
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn cancellation(&self) -> &Cancellation {
        &self.0.cancellation
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancellation().is_requested()
    }
    pub fn request_cancel(&self) -> bool {
        self.cancellation().request() || self.is_cancelled()
    }
    pub fn publish(&self, progress: Progress) {
        *self.0.progress.lock().unwrap() = Some(progress);
    }
    pub fn take_progress(&self) -> Option<Progress> {
        let progress = self.0.progress.lock().unwrap().take();
        if self.is_cancelled() { None } else { progress }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkstone_core::vault::sync::Phase;
    #[test]
    fn workers_coalesce_progress_and_cancel_without_affecting_new_runs() {
        let old = Run::default();
        let worker = old.clone();
        let current = Run::default();
        assert!(old.same(&worker));
        assert!(!current.same(&worker));
        std::thread::spawn(move || {
            for completed in 1..=1000 {
                worker.publish(Progress {
                    phase: Phase::Uploading,
                    completed,
                    total: 1000,
                    bytes: completed as u64,
                });
            }
        })
        .join()
        .unwrap();
        assert_eq!(old.take_progress().unwrap().completed, 1000);
        assert!(old.take_progress().is_none());
        assert!(current.take_progress().is_none());
        assert!(old.request_cancel());
        assert!(old.request_cancel());
        old.publish(Progress {
            phase: Phase::Scanning,
            completed: 1,
            total: 1,
            bytes: 0,
        });
        assert!(old.take_progress().is_none());
        assert!(!current.is_cancelled());
    }
}
