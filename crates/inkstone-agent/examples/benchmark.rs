//! Reproducible synthetic Agent retrieval benchmark; never opens a user's vault.
use inkstone_agent::{Scope, ToolHost};
use inkstone_core::vault::Vault;
use serde_json::json;
use std::time::Instant;
fn main() -> anyhow::Result<()> {
    let count = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000)
        .clamp(1, 10000);
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("notes");
    std::fs::create_dir(&root)?;
    for i in 0..count {
        std::fs::write(
            root.join(format!("note-{i}.md")),
            format!(
                "# 项目 {i}\n\n#知识管理\n\n[[note-{}]]\n\n{}",
                (i + 1) % count,
                "## 研究记录\n\n讨论笔记检索、标签维护和版本恢复。\n\n".repeat(20)
            ),
        )?;
    }
    let vault = Vault::open(&root, dir.path().join("history"))?;
    let start = Instant::now();
    let mut host = ToolHost::new(vault.clone(), Scope::Vault, vec![], None)?;
    let build_ms = start.elapsed().as_millis();
    let start = Instant::now();
    let hits = host.call("search_notes", json!({"query":"版本恢复","limit":20}))?;
    let search_ms = start.elapsed().as_millis();
    let start = Instant::now();
    let single = ToolHost::new(vault, Scope::Notes(vec!["note-0.md".into()]), vec![], None)?;
    println!(
        "{}",
        json!({"notes":count,"scoped_notes":single.index.notes.len(),"build_ms":build_ms,"search_ms":search_ms,"single_note_ms":start.elapsed().as_millis(),"hits":hits["matches"].as_array().map(Vec::len)})
    );
    Ok(())
}
