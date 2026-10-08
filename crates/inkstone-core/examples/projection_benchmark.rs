//! Same-host reading/fragment preparation. Use generated performance corpus only.
use inkstone_core::{index::Index, preview, rendering, syntax::Snapshot};
use sha2::{Digest, Sha256};
use std::{hint::black_box, path::Path, sync::Arc, time::Instant};

fn main() {
    let file = std::env::args()
        .nth(1)
        .expect("usage: projection_benchmark GENERATED_FILE");
    let source = std::fs::read_to_string(file).unwrap();
    let snapshot = Arc::new(Snapshot::new(&source));
    let index = Index::default();
    let path = Path::new("benchmark.md");
    let mut timings = Vec::new();
    let mut signature = None;
    for iteration in 0..25 {
        let start = Instant::now();
        let reading = rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let projected = Instant::now();
        let fragments = preview::fragments(&index, path, snapshot.clone(), &reading);
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let current = (
            format!("{:x}", Sha256::digest(reading.markdown.as_bytes())),
            reading.tasks.len(),
            fragments.len(),
        );
        assert!(signature.as_ref().is_none_or(|old| old == &current));
        signature = Some(current);
        if iteration >= 5 {
            timings.push((
                elapsed,
                projected.duration_since(start).as_secs_f64() * 1000.,
            ));
        }
        black_box((reading, fragments));
    }
    timings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (output_sha256, tasks, fragments) = signature.unwrap();
    println!(
        "profile={} source_sha256={:x} output_sha256={output_sha256} bytes={} tasks={tasks} fragments={fragments} samples=20 total_p50_ms={:.3} total_p95_ms={:.3} reading_at_p50_ms={:.3}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        Sha256::digest(source.as_bytes()),
        source.len(),
        timings[9].0,
        timings[18].0,
        timings[9].1
    );
}
