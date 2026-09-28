//! Reproducible backend measurements. This does not measure interactive latency.
use inkstone::{
    index::{self, Index},
    vault::Vault,
};
use std::{
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
fn main() {
    let count = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10_000);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inkstone-benchmark-{stamp}"));
    let notes = root.join("vault");
    fs::create_dir_all(&notes).unwrap();
    let sample = "# 中文笔记\n\n这是一篇用于搜索测试的本地 Markdown 文档。English 和 emoji 😀。\n\n**重要内容** 与 `代码`，[[00001]]。\n";
    let setup = Instant::now();
    for i in 0..count {
        fs::write(notes.join(format!("{i:05}.md")), sample).unwrap();
    }
    let vault = Vault::open(&notes, root.join("recovery")).unwrap();
    println!(
        "profile=release backend_only=true root={} notes={} each_bytes={} setup_ms={}",
        notes.display(),
        count,
        sample.len(),
        setup.elapsed().as_millis()
    );
    let start = Instant::now();
    let index = Index::build(&vault).unwrap();
    println!("index_build_ms={}", start.elapsed().as_millis());
    let start = Instant::now();
    let hits = index.search("搜索测试");
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
    let no_match = index.search("不存在的查询xyz");
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
