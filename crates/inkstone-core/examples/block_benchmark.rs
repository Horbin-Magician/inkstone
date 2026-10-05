//! Same-input parser CPU comparison, not native input or presentation latency.
use inkstone_core::syntax::Snapshot;
use std::{hint::black_box, time::Instant};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: block_benchmark FILE MARKER");
    let source = std::fs::read_to_string(&args[1]).unwrap();
    let at = source.find(&args[2]).expect("marker missing") + args[2].len();
    let base = Snapshot::new(&source);
    let mut full_ms = Vec::new();
    let mut local_ms = Vec::new();
    for i in 0..25 {
        let mut text = source.clone();
        text.insert_str(at, &format!("中文{i}"));
        let start = Instant::now();
        let full = black_box(Snapshot::new(black_box(&text)));
        let full_time = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let local = black_box(
            base.update_block(black_box(&text))
                .expect("not an incremental edit"),
        );
        let local_time = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(local.ast, full.ast);
        if i >= 5 {
            full_ms.push(full_time);
            local_ms.push(local_time);
        }
    }
    full_ms.sort_by(f64::total_cmp);
    local_ms.sort_by(f64::total_cmp);
    println!(
        "bytes={} samples=20 full_p50_ms={:.3} full_p95_ms={:.3} block_p50_ms={:.3} block_p95_ms={:.3}",
        source.len(),
        full_ms[9],
        full_ms[18],
        local_ms[9],
        local_ms[18]
    );
}
