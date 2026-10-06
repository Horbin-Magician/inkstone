//! Compare legacy journal decoding in separate processes to measure peak RSS.
//! cargo run --release -p inkstone-core --example history_read_benchmark -- MODE FILE
//! MODE is `whole`, `buffered`, or `metadata` (current cold-import reader).
#[path = "../src/vault/history/metadata.rs"]
mod metadata;
use inkstone_core::vault::Recovery;
use std::{fs, hint::black_box, io, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected MODE FILE; MODE is whole, buffered or metadata".into());
    }
    let start = Instant::now();
    if args[0] == "metadata" {
        let record: metadata::Metadata = serde_json::from_reader(io::BufReader::with_capacity(
            64 * 1024,
            fs::File::open(&args[1])?,
        ))?;
        println!(
            "mode=metadata elapsed_ms={:.3} root={} relative={}",
            start.elapsed().as_secs_f64() * 1000.,
            record.root.display(),
            record.relative.display()
        );
        black_box(&record);
        return Ok(());
    }
    let record: Recovery = match args[0].to_str() {
        Some("whole") => serde_json::from_slice(&fs::read(&args[1])?)?,
        Some("buffered") => serde_json::from_reader(io::BufReader::with_capacity(
            64 * 1024,
            fs::File::open(&args[1])?,
        ))?,
        _ => return Err("MODE must be whole or buffered".into()),
    };
    let elapsed = start.elapsed();
    println!(
        "mode={} elapsed_ms={:.3} baseline_bytes={} draft_bytes={}",
        args[0].to_string_lossy(),
        elapsed.as_secs_f64() * 1000.,
        record.baseline.as_ref().map_or(0, String::len),
        record.draft.len()
    );
    black_box(&record);
    Ok(())
}
