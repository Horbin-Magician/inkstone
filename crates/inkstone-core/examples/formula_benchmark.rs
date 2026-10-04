//! Same-host backend timings, excluding graphics, UI layout, and native input latency.
use inkstone_core::{index::Index, preview, rendering, syntax::Snapshot};
use std::{path::Path, sync::Arc, time::Instant};

fn main() {
    let index = Index::default();
    let path = Path::new("formulas.md");
    for count in [100, 500, 1000] {
        let source = (0..count)
            .map(|i| format!("第{i}行 $\\frac{{x_{{{i}}}^2}}{{1+x_{{{i}}}}}$\n\n"))
            .collect::<String>();
        let mut samples = vec![];
        for _ in 0..3 {
            let start = Instant::now();
            let snapshot = Arc::new(Snapshot::new(&source));
            let reading =
                rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
            let prepared = Instant::now();
            let fragments = preview::fragments(&index, path, snapshot, &reading);
            let end = Instant::now();
            assert_eq!(fragments.len(), count);
            assert!(fragments.iter().all(|fragment| fragment.graphic.is_some()));
            samples.push((end - start, end - prepared));
            std::hint::black_box(fragments);
        }
        samples.sort_by_key(|sample| sample.0);
        println!(
            "profile={} formulas={count} rounds=3 total_p50_ms={:.3} fragments_ms={:.3}",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            samples[1].0.as_secs_f64() * 1000.,
            samples[1].1.as_secs_f64() * 1000.,
        );
    }
}
