//! Opt-in release diagnostics; never record document contents.
use gpui::{App, Window};
use std::{path::PathBuf, sync::OnceLock, time::Instant};

static START: OnceLock<Instant> = OnceLock::new();
static READY_MS: OnceLock<f64> = OnceLock::new();
static OUTPUT: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn init() {
    START.get_or_init(Instant::now);
    OUTPUT.get_or_init(|| std::env::var_os("INKSTONE_METRICS_PATH").map(PathBuf::from));
}

pub fn needs_ready() -> bool {
    OUTPUT.get().is_some_and(Option::is_some) && READY_MS.get().is_none()
}

pub fn record_ready() {
    if let Some(start) = START.get() {
        let _ = READY_MS.set(start.elapsed().as_secs_f64() * 1000.);
    }
}

pub fn sample(window: &Window, files: usize, document_bytes: usize, cx: &App) {
    let Some(Some(path)) = OUTPUT.get() else {
        return;
    };
    let input = window.input_latency_snapshot();
    let frame = window.frame_duration_snapshot();
    let values = serde_json::json!({
        "schema": 1,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "measurement": "GPUI input dispatch to platform frame submission; not hardware/display latency",
        "elapsed_ms": START.get().map(|start| start.elapsed().as_secs_f64() * 1000.),
        "first_editor_frame_callback_ms": READY_MS.get(),
        "files": files,
        "document_bytes": document_bytes,
        "scale_factor": window.scale_factor(),
        "input_to_frame": {
            "count": input.latency_histogram.len(),
            "p50_ms": input.latency_histogram.value_at_quantile(0.50) as f64 / 1_000_000.,
            "p95_ms": input.latency_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
            "max_ms": input.latency_histogram.max() as f64 / 1_000_000.,
            "mid_draw_excluded": input.mid_draw_events_dropped
        },
        "draw": {
            "count": frame.draw_duration_histogram.len(),
            "p50_ms": frame.draw_duration_histogram.value_at_quantile(0.50) as f64 / 1_000_000.,
            "p95_ms": frame.draw_duration_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
            "max_ms": frame.draw_duration_histogram.max() as f64 / 1_000_000.
        },
        "animation_present_interval": {
            "count": frame.present_interval_histogram.len(),
            "p95_ms": frame.present_interval_histogram.value_at_quantile(0.95) as f64 / 1_000_000.
        }
    });
    let path = path.clone();
    cx.background_executor()
        .spawn(async move {
            if let Ok(bytes) = serde_json::to_vec_pretty(&values) {
                let _ = std::fs::write(path, bytes);
            }
        })
        .detach();
}
