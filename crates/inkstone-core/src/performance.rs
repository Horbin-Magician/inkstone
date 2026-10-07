//! Bounded, opt-in diagnostic wall-clock spans. Not native frame/input timings.
//! Set INKSTONE_TRACE_PERFORMANCE to a new JSONL file in an existing directory.
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const LIMIT: usize = 10_000;
static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    SyntaxFull,
    SyntaxIncremental,
    ReadingProjection,
    PreviewFragments,
    PresentationUpdate,
    EditorViewComposition,
    IndexLoad,
    FulltextSearch,
    FilenameSearch,
    FileRefresh,
    SaveTransaction,
    SyncTransaction,
    SyncScan,
}
struct Recorder {
    origin: Instant,
    unix_origin_ms: u128,
    count: AtomicUsize,
    output: Mutex<Option<File>>,
}
impl Recorder {
    fn reserve(&self) -> Option<usize> {
        self.count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < LIMIT).then_some(count + 1)
            })
            .ok()
    }
}
#[derive(Serialize)]
struct Record {
    schema_version: u8,
    pid: u32,
    sequence: usize,
    unix_origin_ms: u128,
    stage: Stage,
    start_us: u128,
    duration_us: u128,
}

#[must_use]
pub struct Span(Option<(&'static Recorder, Stage, Instant)>);

pub fn span(stage: Stage) -> Span {
    let recorder = RECORDER.get_or_init(|| {
        let path = std::env::var_os("INKSTONE_TRACE_PERFORMANCE")?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .ok()?;
        Some(Recorder {
            origin: Instant::now(),
            unix_origin_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            count: AtomicUsize::new(0),
            output: Mutex::new(Some(file)),
        })
    });
    Span(
        recorder
            .as_ref()
            .filter(|r| r.count.load(Ordering::Relaxed) < LIMIT)
            .map(|r| (r, stage, Instant::now())),
    )
}
impl Drop for Span {
    fn drop(&mut self) {
        let Some((recorder, stage, start)) = self.0 else {
            return;
        };
        let duration_us = start.elapsed().as_micros();
        let Some(sequence) = recorder.reserve() else {
            return;
        };
        let record = Record {
            schema_version: 1,
            pid: std::process::id(),
            sequence,
            unix_origin_ms: recorder.unix_origin_ms,
            stage,
            start_us: start.duration_since(recorder.origin).as_micros(),
            duration_us,
        };
        // No user-provided stage names, file paths, query text or document data.
        // Diagnostic writes have overhead; normal budget runs leave tracing off.
        // A failed sink is retired without changing application behavior.
        if let Ok(mut output) = recorder.output.lock()
            && let Some(file) = output.as_mut()
            && (serde_json::to_writer(&mut *file, &record).is_err()
                || file.write_all(b"\n").is_err())
        {
            *output = None;
            recorder.count.store(LIMIT, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_schema_is_content_free_and_budget_is_bounded() {
        let recorder = Recorder {
            origin: Instant::now(),
            unix_origin_ms: 0,
            count: AtomicUsize::new(LIMIT - 1),
            output: Mutex::new(None),
        };
        assert_eq!(recorder.reserve(), Some(LIMIT - 1));
        assert_eq!(recorder.reserve(), None);
        let value = serde_json::to_value(Record {
            schema_version: 1,
            pid: 1,
            sequence: 0,
            unix_origin_ms: 0,
            stage: Stage::SyntaxFull,
            start_us: 10,
            duration_us: 20,
        })
        .unwrap();
        assert_eq!(value["stage"], "syntax_full");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value.as_object().unwrap().len(), 7);
    }
}
