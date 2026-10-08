//! Cancellation owned by the pane, independently of a running worker's lifetime.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct Job(Arc<AtomicBool>);
impl Job {
    pub fn token(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_or_dropping_owner_cancels_only_its_worker() {
        let mut owner = Some(Job::default());
        let first = owner.as_ref().unwrap().token();
        owner = Some(Job::default());
        let latest = owner.as_ref().unwrap().token();
        assert!(first.load(Ordering::Acquire));
        assert!(!latest.load(Ordering::Acquire));
        drop(owner);
        assert!(latest.load(Ordering::Acquire));
    }
}
