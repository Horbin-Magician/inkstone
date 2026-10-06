//! Generated history only; separates legacy import, catalog scans and note caches.
//! cargo run --release --locked -p inkstone-core --example history_catalog_benchmark -- NEW_DIR
use inkstone_core::vault::Vault;
use std::{fs, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("provide a fresh output directory")?,
    );
    // Refuse existing directories/symlinks rather than touching a real history store.
    fs::create_dir(&root)?;
    let root = fs::canonicalize(root)?;
    fs::create_dir(root.join("vault"))?;
    let vault = Vault::open(root.join("vault"), root.join("recovery"))?;
    let body = "中文 😀 é 历史正文\n".repeat(128);
    let notes = 1000;
    let versions = 10;
    let mut bytes = 0u64;
    for note in 0..notes {
        for version in 0..versions {
            let record = serde_json::to_vec(&serde_json::json!({
                "root": vault.root,
                "relative": format!("目录/笔记-{note}.md"),
                "baseline": null,
                "draft": body,
            }))?;
            bytes += record.len() as u64;
            fs::write(
                root.join("recovery")
                    .join(format!("{note}-{version}.saved")),
                record,
            )?;
        }
    }
    let mut samples = Vec::new();
    for round in 0..4 {
        let start = Instant::now();
        let catalog = vault.history_notes()?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(catalog.len(), notes);
        assert!(catalog.iter().all(|n| n.records == versions));
        assert_eq!(catalog.iter().map(|n| n.bytes).sum::<u64>(), bytes);
        samples.push(serde_json::json!({"operation": "catalog", "round": round,
            "cache": if round == 0 { "cold" } else { "warm" }, "elapsed_ms": elapsed_ms}));
    }
    for round in 0..4 {
        let start = Instant::now();
        let entries = vault.history(std::path::Path::new("目录/笔记-0.md"))?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(entries.len(), versions);
        samples.push(serde_json::json!({"operation": "note", "round": round,
            "cache": if round == 0 { "record metadata only" } else { "note index" },
            "elapsed_ms": elapsed_ms}));
        // Body loading is outside the timed metadata operation.
        assert_eq!(
            vault
                .read_history(std::path::Path::new("目录/笔记-0.md"), &entries[0].journal)?
                .draft,
            body
        );
    }
    let report = serde_json::json!({"notes": notes, "versions_each": versions,
        "journal_bytes": bytes, "samples": samples,
        "scope": "backend wall time; cold means no metadata cache, not cold OS page cache"});
    fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
