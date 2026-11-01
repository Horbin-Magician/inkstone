//! Explicit live smoke test: reads a test API key from stdin, never from argv.
//! Sends only synthetic data; makes two small, billable requests.
use anyhow::{Result, ensure};
use inkstone_agent::{ModelConfig, ModelProvider, provider::CompatibleProvider, runtime::Cancel};
use serde_json::json;
use std::io::Read;

fn main() -> Result<()> {
    let mut key = String::new();
    std::io::stdin().read_to_string(&mut key)?;
    let provider = CompatibleProvider {
        config: ModelConfig {
            output_tokens: 2048,
            ..Default::default()
        },
        key: key.trim().into(),
    };
    ensure!(!provider.key.is_empty(), "Provide a test key via stdin");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let cancel = Cancel::default();
        let tools = vec![json!({"type":"function","function":{"name":"inkstone_probe","description":"Synthetic test; no side effects","parameters":{"type":"object","properties":{}}}})];
        let mut messages = vec![json!({"role":"user","content":"Call inkstone_probe once with {}. After its result, reply only PROBE_OK."})];
        let first = provider.complete(&messages, &tools, &cancel, &mut |_| Ok(())).await?;
        ensure!(first.calls.len() == 1 && first.calls[0].name == "inkstone_probe", "Expected a synthetic tool call");
        messages.push(json!({"role":"assistant","response_items":first.output_items}));
        messages.push(json!({"role":"tool","tool_call_id":first.calls[0].id,"content":"{\"ok\":true}"}));
        let second = provider.complete(&messages, &tools, &cancel, &mut |_| Ok(())).await?;
        ensure!(second.calls.is_empty() && second.text.contains("PROBE_OK"), "Expected final acknowledgment");
        println!("PASS: Responses streaming, tool call, result replay, final acknowledgment");
        Ok(())
    })
}
