//! Sync queue policy, independent of GPUI. W owns a cancellable timer handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WaitKind {
    Quiet,
    Retry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pump {
    Wait(WaitKind),
    Reschedule,
    Idle,
}

pub(super) struct Schedule<W> {
    pub(super) pending: bool,
    /// Whether the pending request may be delayed by quiet/retry timers.
    pub(super) automatic: bool,
    /// Changes arriving during an active attempt, or a failed attempt to retry.
    pub(super) again: bool,
    /// Timer creation is deferred to the window adapter on the next tick.
    pub(super) defer_quiet: bool,
    pub(super) defer_retry: bool,
    /// Dropping/replacing W cancels the adapter-owned wait.
    pub(super) wait: Option<(WaitKind, W)>,
    pub(super) busy: bool,
}
impl<W> Default for Schedule<W> {
    fn default() -> Self {
        Self {
            pending: false,
            automatic: false,
            again: false,
            defer_quiet: false,
            defer_retry: false,
            wait: None,
            busy: false,
        }
    }
}
impl<W> Schedule<W> {
    pub fn retry_waiting(&self) -> bool {
        self.defer_retry
            || self
                .wait
                .as_ref()
                .is_some_and(|(kind, _)| *kind == WaitKind::Retry)
    }
    pub fn queued(&self) -> bool {
        self.pending || self.busy || self.again || self.defer_retry
    }
    pub fn ready(&self) -> bool {
        self.pending
            && !self.busy
            && !(self.automatic && (self.wait.is_some() || self.defer_quiet || self.defer_retry))
    }
    pub fn cancel_queued(&mut self) {
        self.pending = false;
        self.automatic = false;
        self.again = false;
        self.force_ready();
    }
    pub fn force_ready(&mut self) {
        self.defer_quiet = false;
        self.defer_retry = false;
        self.wait = None;
    }
    pub fn automatic(&mut self, quiet: bool) {
        if self.busy {
            self.again = true;
            return;
        }
        if self.pending && !self.automatic {
            return;
        }
        self.pending = true;
        self.automatic = true;
        if quiet && !self.retry_waiting() {
            self.defer_quiet = true;
        }
    }
    pub fn manual(&mut self) {
        self.cancel_queued();
        self.pending = true;
    }
    pub fn disable_automatic(&mut self) {
        self.again = false;
        self.force_ready();
        if self.automatic {
            self.clear_request();
        }
    }
    pub fn clear_request(&mut self) {
        self.pending = false;
        self.automatic = false;
    }
    pub fn blocked(&mut self, retry: bool) {
        self.clear_request();
        self.defer_quiet = false;
        self.wait = None;
        if retry {
            self.retry();
        }
    }
    pub fn retry(&mut self) {
        self.again = true;
        self.defer_retry = true;
    }
    pub fn defer_retry(&mut self) {
        self.defer_retry = true;
    }
    pub fn start(&mut self) {
        self.cancel_queued();
        self.busy = true;
    }
    pub fn start_connection_test(&mut self) {
        self.busy = true;
    }
    pub fn finish(&mut self) {
        self.busy = false;
    }
    pub fn take_again(&mut self) -> bool {
        std::mem::take(&mut self.again)
    }
    pub fn pump(&mut self) -> Pump {
        if self.defer_quiet {
            self.defer_quiet = false;
            self.defer_retry = false;
            Pump::Wait(WaitKind::Quiet)
        } else if self.defer_retry && self.wait.is_none() {
            self.defer_retry = false;
            Pump::Wait(WaitKind::Retry)
        } else if self.again && !self.busy && self.wait.is_none() {
            self.again = false;
            Pump::Reschedule
        } else {
            Pump::Idle
        }
    }
    pub fn set_wait(&mut self, kind: WaitKind, handle: W) {
        self.wait = Some((kind, handle));
    }
    pub fn elapsed(&mut self) {
        self.wait = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    struct Timer(Rc<Cell<usize>>);
    impl Drop for Timer {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    #[test]
    fn quiet_replacement_manual_override_and_reset_drop_timer_handles() {
        let dropped = Rc::new(Cell::new(0));
        let mut queue = Schedule::default();
        queue.automatic(true);
        assert_eq!(queue.pump(), Pump::Wait(WaitKind::Quiet));
        queue.set_wait(WaitKind::Quiet, Timer(dropped.clone()));
        assert!(!queue.ready());
        queue.automatic(true);
        assert_eq!(queue.pump(), Pump::Wait(WaitKind::Quiet));
        queue.set_wait(WaitKind::Quiet, Timer(dropped.clone()));
        assert_eq!(dropped.get(), 1);
        queue.manual();
        assert_eq!(dropped.get(), 2);
        queue.automatic(true);
        assert!(queue.ready());
        assert!(!queue.automatic);
        queue.disable_automatic();
        assert!(
            queue.ready(),
            "turning off auto must retain the manual request"
        );
        queue.start();
        queue.finish();
        queue.automatic(true);
        assert_eq!(queue.pump(), Pump::Wait(WaitKind::Quiet));
        queue.set_wait(WaitKind::Quiet, Timer(dropped.clone()));
        queue = Schedule::default(); // Vault/account reset.
        assert_eq!(dropped.get(), 3);
        assert!(!queue.queued());
    }
    #[test]
    fn local_changes_do_not_shorten_retry_and_success_resumes_followup() {
        let mut queue = Schedule::<()>::default();
        queue.manual();
        queue.start();
        queue.automatic(true);
        queue.finish();
        queue.retry();
        assert!(queue.take_again());
        queue.automatic(false);
        queue.automatic(true);
        assert!(!queue.defer_quiet);
        assert_eq!(queue.pump(), Pump::Wait(WaitKind::Retry));
        queue.set_wait(WaitKind::Retry, ());
        queue.automatic(true);
        assert_eq!(queue.pump(), Pump::Idle);
        assert!(!queue.ready());
        queue.elapsed();
        assert!(queue.ready());
        queue.start();
        queue.automatic(false);
        queue.finish();
        assert_eq!(queue.pump(), Pump::Reschedule);
        queue.automatic(false);
        assert!(queue.ready());
    }
    #[test]
    fn cancellation_keeps_running_protection_until_completion() {
        let mut queue = Schedule::<()>::default();
        queue.manual();
        queue.start();
        queue.automatic(true);
        queue.cancel_queued();
        assert!(queue.busy);
        assert!(!queue.pending && !queue.again);
        queue.finish();
        assert_eq!(queue.pump(), Pump::Idle);
        queue.automatic(false);
        queue.blocked(true);
        assert!(!queue.ready());
        assert_eq!(queue.pump(), Pump::Wait(WaitKind::Retry));
        queue.set_wait(WaitKind::Retry, ());
        queue.force_ready(); // Window close bypasses timers, but never local save checks.
        assert_eq!(queue.pump(), Pump::Reschedule);
        queue.automatic(false);
        assert!(queue.ready());
    }
}
