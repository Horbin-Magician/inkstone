//! Keep network work alive independently of the workspace and its window.
use gpui::{App, Global, Task};

#[derive(Default)]
struct BackgroundSync {
    active: usize,
    window_closed: bool,
}

impl Global for BackgroundSync {}

impl BackgroundSync {
    fn ready_to_quit(&self) -> bool {
        self.window_closed && self.active == 0
    }
}

pub fn window_closed(cx: &mut App) {
    let state = cx.default_global::<BackgroundSync>();
    state.window_closed = true;
    if state.ready_to_quit() {
        cx.quit();
    }
}

/// This collector belongs to the application, so completion does not require a
/// live window or workspace. Errors also release the process normally.
pub fn retain<T: 'static>(task: Task<T>, cx: &mut App) -> Task<T> {
    cx.default_global::<BackgroundSync>().active += 1;
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let state = cx.global_mut::<BackgroundSync>();
            state.active -= 1;
            if state.ready_to_quit() {
                cx.quit();
            }
        });
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[gpui::test]
    fn closed_window_waits_for_all_tasks_including_failures(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for (seconds, result) in [(1, Ok(())), (2, Err("network failure"))] {
                let timer = cx.background_executor().timer(Duration::from_secs(seconds));
                let task = cx.background_executor().spawn(async move {
                    timer.await;
                    result
                });
                retain(task, cx).detach();
            }
            window_closed(cx);
            assert!(!cx.global::<BackgroundSync>().ready_to_quit());
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(cx.global::<BackgroundSync>().active, 1);
            assert!(!cx.global::<BackgroundSync>().ready_to_quit());
        });
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|cx| assert!(cx.global::<BackgroundSync>().ready_to_quit()));
    }

    #[gpui::test]
    fn completed_sync_keeps_an_open_window_running(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let task = cx.background_executor().spawn(async {});
            retain(task, cx).detach();
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(cx.global::<BackgroundSync>().active, 0);
            assert!(!cx.global::<BackgroundSync>().ready_to_quit());
            window_closed(cx);
            assert!(cx.global::<BackgroundSync>().ready_to_quit());
        });
    }
}
