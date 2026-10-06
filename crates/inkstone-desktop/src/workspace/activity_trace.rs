//! Opt-in, bounded native acceptance diagnostics. Never includes document data.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const MAX_SAMPLES: usize = 1800;

pub(super) struct ActivityTrace {
    file: File,
    started: Instant,
    samples: usize,
}

impl ActivityTrace {
    pub fn from_env() -> Option<Self> {
        if cfg!(test) {
            return None;
        }
        let path = std::env::var_os("INKSTONE_TRACE_ACTIVITY")?;
        match Self::create(Path::new(&path)) {
            Ok(trace) => Some(trace),
            Err(error) => {
                eprintln!("Activity trace disabled: {error}");
                None
            }
        }
    }

    fn create(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().write(true).create_new(true).open(path)?,
            started: Instant::now(),
            samples: 0,
        })
    }

    /// False disables tracing after an error or the one-hour sample budget.
    pub fn record(&mut self, active: bool, focused: bool, loading: bool) -> bool {
        if self.samples >= MAX_SAMPLES {
            return false;
        }
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let result = writeln!(
            self.file,
            "{{\"pid\":{},\"unix_ms\":{},\"elapsed_ms\":{},\"window_active\":{},\"editor_focused\":{},\"loading\":{}}}",
            std::process::id(),
            unix_ms,
            self.started.elapsed().as_millis(),
            active,
            focused,
            loading,
        );
        self.samples += 1;
        if let Err(error) = result {
            eprintln!("Activity trace stopped: {error}");
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_trace_is_bounded_and_does_not_overwrite_existing_files() {
        let root = std::env::temp_dir().join(format!(
            "inkstone-activity-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("activity.jsonl");
        let mut trace = ActivityTrace::create(&path).unwrap();
        assert!(trace.record(true, false, true));
        assert!(trace.record(false, true, false));
        let bytes = std::fs::read(&path).unwrap();
        assert!(ActivityTrace::create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let records: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["window_active"], true);
        assert_eq!(records[0]["editor_focused"], false);
        assert_eq!(records[0]["loading"], true);
        assert_eq!(records[1]["window_active"], false);
        assert_eq!(records[1]["editor_focused"], true);
        assert_eq!(records[1].as_object().unwrap().len(), 6);
        trace.samples = MAX_SAMPLES;
        assert!(!trace.record(true, true, false));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        drop(trace);
        std::fs::remove_dir_all(root).unwrap();
    }
}
