//! Reproducible backend measurements. This does not measure interactive latency.
use inkstone_core::{
    index::{self, Index},
    vault::Vault,
};
use std::{
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn measure<T>(name: &str, mut run: impl FnMut() -> T) {
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        let result = std::hint::black_box(run());
        samples.push(start.elapsed().as_secs_f64() * 1000.);
        drop(result);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name} rounds=7 p50_ms={:.3} max_ms={:.3}",
        samples[3], samples[6]
    );
}

fn main() {
    let count = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10_000);
    let repetitions = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-benchmark-{stamp}"));
    let notes = root.join("vault");
    fs::create_dir_all(&notes).unwrap();
    let sample = "# 中文笔记\n\n这是一篇用于搜索测试的本地 Markdown 文档。English 和 emoji 😀。\n\n**重要内容** 与 `代码`，[[00001]]。\n".repeat(repetitions);
    let setup = Instant::now();
    for i in 0..count {
        fs::write(notes.join(format!("{i:05}.md")), &sample).unwrap();
    }
    let vault = Vault::open(&notes, root.join("recovery")).unwrap();
    println!(
        "profile={} backend_only=true root={} notes={} each_bytes={} setup_ms={}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        notes.display(),
        count,
        sample.len(),
        setup.elapsed().as_millis()
    );
    let start = Instant::now();
    let index = Index::build(&vault).unwrap();
    println!("index_build_ms={}", start.elapsed().as_millis());
    // Compare the previous owned-note representation with shared snapshots on
    // identical data, in the same process. Drop work is outside the timed region.
    let owned_notes: std::collections::BTreeMap<_, _> = index
        .notes
        .iter()
        .map(|(path, note)| (path.clone(), note.as_ref().clone()))
        .collect();
    let mut owned_samples = Vec::new();
    let mut shared_samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        let copy = std::hint::black_box((
            owned_notes.clone(),
            index.files.clone(),
            index.errors.clone(),
        ));
        owned_samples.push(start.elapsed().as_secs_f64() * 1000.);
        drop(copy);
        let start = Instant::now();
        let copy = std::hint::black_box(index.clone());
        shared_samples.push(start.elapsed().as_secs_f64() * 1000.);
        drop(copy);
    }
    owned_samples.sort_by(f64::total_cmp);
    shared_samples.sort_by(f64::total_cmp);
    println!(
        "snapshot_clone_rounds=7 owned_p50_ms={:.3} shared_p50_ms={:.3}",
        owned_samples[3], shared_samples[3]
    );
    let start = Instant::now();
    let hits = index
        .search_limited("搜索测试", false, Default::default(), false, 200)
        .unwrap();
    println!(
        "search_ms={:.3} hits={} cap=200",
        start.elapsed().as_secs_f64() * 1000.,
        hits.len()
    );
    let start = Instant::now();
    let refs = index.backlinks(&PathBuf::from("00001.md"));
    println!(
        "backlinks_ms={:.3} count={}",
        start.elapsed().as_secs_f64() * 1000.,
        refs.len()
    );
    let start = Instant::now();
    let no_match = index
        .search_limited("不存在的查询xyz", false, Default::default(), false, 200)
        .unwrap();
    println!(
        "search_no_match_ms={:.3} hits={}",
        start.elapsed().as_secs_f64() * 1000.,
        no_match.len()
    );
    fs::write(notes.join("00001.md"), format!("{sample}\n修改的一行")).unwrap();
    let start = Instant::now();
    let mut incremental = index.clone();
    incremental
        .refresh_paths(&vault, [PathBuf::from("00001.md")])
        .unwrap();
    println!(
        "single_file_refresh_including_index_clone_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    for (name, query) in [
        ("search_common", "搜索测试"),
        ("search_absent", "不存在的查询xyz"),
        ("search_property", "-[status]"),
    ] {
        measure(name, || {
            index
                .search_limited(query, false, Default::default(), false, 200)
                .unwrap()
        });
    }
    measure("quick_open_common", || index.filenames("0"));
    measure("snapshot_clone", || index.clone());
    let mut revision = 0;
    measure("body_update", || {
        revision += 1;
        incremental.update("00001.md".into(), format!("{sample}\nrevision {revision}"));
    });
    measure("link_update", || {
        revision += 1;
        incremental.update(
            "00001.md".into(),
            format!("[[{:05}]]", revision % count.max(1)),
        );
    });
    let cache = root.join("index.json");
    measure("cache_save", || index.save_cache(&vault, &cache).unwrap());
    println!("cache_bytes={}", fs::metadata(&cache).unwrap().len());
    measure("cache_load", || {
        Index::build_cached(&vault, &cache).unwrap()
    });
    let large = format!(
        "# 长单段\n\n{}",
        "中文 English 😀 超长段落。".repeat(100_000)
    );
    let start = Instant::now();
    let parsed = index::parse(&large);
    println!(
        "single_document_bytes={} parse_ms={} headings={}",
        large.len(),
        start.elapsed().as_millis(),
        parsed.headings.len()
    );
    // Delete only this process's uniquely created benchmark directory.
    fs::remove_dir_all(root).unwrap();
}
