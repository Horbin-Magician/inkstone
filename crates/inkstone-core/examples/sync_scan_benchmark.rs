//! Measure the real sync scan, cancelling before any remote request or publication.
use anyhow::Result;
use inkstone_core::vault::{
    Vault,
    sync::{self, Cancellation, Manifest, Phase, Remote},
};
use std::{sync::Mutex, time::Instant};

struct NoRemote;
impl Remote for NoRemote {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        unreachable!("scan benchmark reached remote")
    }
    fn download(&self, _: &str) -> Result<Vec<u8>> {
        unreachable!("scan benchmark downloaded")
    }
    fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
        unreachable!("scan benchmark uploaded")
    }
    fn publish(&self, _: &Manifest, _: Option<&str>) -> Result<()> {
        unreachable!("scan benchmark published")
    }
}
#[derive(Default)]
struct Timing {
    start: Option<Instant>,
    enumerated: Option<Instant>,
    processed: Option<Instant>,
    finished: Option<Instant>,
    files: usize,
    bytes: u64,
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: sync_scan_benchmark GENERATED_VAULT NEW_RECOVERY_DIR"
    );
    // Never reuse recovery data: sync setup may remove its own partial downloads.
    std::fs::create_dir(&args[2])?;
    let vault = Vault::open(&args[1], &args[2])?;
    let cancellation = Cancellation::default();
    let timing = Mutex::new(Timing::default());
    let result = sync::synchronize_cancellable(
        &vault,
        &NoRemote,
        "scan-benchmark",
        &cancellation,
        |progress| {
            let now = Instant::now();
            let mut timing = timing.lock().unwrap();
            match progress.phase {
                Phase::Scanning => {
                    timing.start.get_or_insert(now);
                    if progress.total > 0 && progress.completed == 0 {
                        timing.enumerated = Some(now);
                    }
                    if progress.total > 0 && progress.completed == progress.total {
                        timing.processed = Some(now);
                        timing.files = progress.total;
                        timing.bytes = progress.bytes;
                    }
                }
                Phase::ReadingManifest => {
                    timing.finished = Some(now);
                    assert!(cancellation.request());
                }
                _ => unreachable!("scan benchmark advanced beyond scan"),
            }
        },
    );
    let error = result.expect_err("scan benchmark must cancel");
    anyhow::ensure!(sync::is_cancelled(&error), "scan failed: {error:#}");
    let timing = timing.into_inner().unwrap();
    let start = timing.start.expect("scan started");
    let enumerated = timing
        .enumerated
        .expect("nonempty generated fixture required");
    let processed = timing.processed.expect("all files processed");
    let finished = timing.finished.expect("scan completed");
    println!(
        "{}",
        serde_json::json!({
            "files": timing.files, "bytes": timing.bytes,
            "enumeration_ms": (enumerated-start).as_secs_f64()*1000.,
            "file_processing_ms": (processed-enumerated).as_secs_f64()*1000.,
            "manifest_validation_ms": (finished-processed).as_secs_f64()*1000.,
            "scan_ms": (finished-start).as_secs_f64()*1000.,
            "cancelled_before_remote": true,
        })
    );
    Ok(())
}
