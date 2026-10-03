//! Opt-in startup milestones, in milliseconds since entering main.
use std::{sync::OnceLock, time::Instant};
static START: OnceLock<Option<Instant>> = OnceLock::new();
pub fn enabled() -> bool {
    START
        .get_or_init(|| std::env::var_os("INKSTONE_TRACE_STARTUP").map(|_| Instant::now()))
        .is_some()
}
pub fn mark(stage: &str) {
    if enabled() {
        let start = START.get().and_then(|start| *start).unwrap();
        eprintln!(
            "[startup] {:10.3} ms {stage}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}
