//! Monotonic remote-check cadence, independent of UI entities and executor tasks.
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct RemoteCheck {
    next: Option<Instant>,
    active: bool,
    last_request: Option<Instant>,
    interval: Option<Duration>,
}
impl RemoteCheck {
    pub fn poll(
        &mut self,
        now: Instant,
        enabled: bool,
        interval: Duration,
        active: bool,
        queued: bool,
    ) -> bool {
        if !enabled {
            *self = Self::default();
            return false;
        }
        if self.next.is_none() || self.interval != Some(interval) {
            self.next = Some(now + interval);
            self.interval = Some(interval);
            self.active = active;
            self.last_request = Some(now);
            return false;
        }
        let activated = active && !self.active;
        self.active = active;
        let due = self.next.is_some_and(|next| now >= next);
        // Rapid app switches do not flood the remote. Existing queued/retry work
        // already observes the remote, so never replace its backoff or quiet wait.
        let activation_due = activated
            && self
                .last_request
                .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(15));
        if due || activation_due {
            self.next = Some(now + interval);
            self.last_request = Some(now);
            return !queued;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_inactive_device_checks_remote_and_coalesces_busy_work() {
        let mut state = RemoteCheck::default();
        let now = Instant::now();
        let interval = Duration::from_secs(300);
        assert!(!state.poll(now, true, interval, false, false));
        assert!(!state.poll(now + Duration::from_secs(299), true, interval, false, false));
        assert!(state.poll(now + interval, true, interval, false, false));
        assert!(!state.poll(now + interval * 2, true, interval, false, true));
        assert!(!state.poll(
            now + interval * 2 + Duration::from_secs(1),
            true,
            interval,
            false,
            false
        ));
        assert!(state.poll(now + interval * 3, true, interval, false, false));
    }
    #[test]
    fn activation_has_cooldown_and_disable_resets_vault_state() {
        let mut state = RemoteCheck::default();
        let now = Instant::now();
        let interval = Duration::from_secs(300);
        assert!(!state.poll(now, true, interval, false, false));
        assert!(state.poll(now + Duration::from_secs(20), true, interval, true, false));
        assert!(!state.poll(now + Duration::from_secs(21), true, interval, false, false));
        assert!(!state.poll(now + Duration::from_secs(22), true, interval, true, false));
        assert!(!state.poll(now + Duration::from_secs(600), false, interval, true, false));
        assert!(!state.poll(now + Duration::from_secs(601), true, interval, true, false));
    }
    #[test]
    fn interval_changes_rearm_without_an_immediate_request() {
        let mut state = RemoteCheck::default();
        let now = Instant::now();
        assert!(!state.poll(now, true, Duration::from_secs(300), true, false));
        assert!(!state.poll(
            now + Duration::from_secs(1),
            true,
            Duration::from_secs(60),
            true,
            false
        ));
        assert!(state.poll(
            now + Duration::from_secs(61),
            true,
            Duration::from_secs(60),
            true,
            false
        ));
    }
}
