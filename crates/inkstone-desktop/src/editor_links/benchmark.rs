//! Manual provider CPU timing; excludes native keyboard, menu layout and drawing.
use super::*;
use gpui::TestAppContext;
use sha2::{Digest, Sha256};
use std::time::Instant;

#[gpui::test]
#[ignore = "manual same-host completion provider CPU comparison"]
async fn completion_provider_performance(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.add_window(|w, cx| crate::editor::EditorPane::new("", w, cx));
    let anchors: Arc<Vec<(String, String)>> = Arc::new(
        (0..40)
            .map(|i| {
                let title = format!("计划 Plan {i:02}");
                (title.clone(), title)
            })
            .collect(),
    );
    let paths: Vec<_> = (0..40)
        .map(|i| CompletionPath {
            label: format!("folder/计划 Plan {i:02}"),
            target: format!("note{i}"),
            alias: (i % 2 == 0).then(|| format!("别名计划 Plan {i:02}")),
            anchors: anchors.clone(),
            ..Default::default()
        })
        .collect();
    let provider = WikiCompletions(Rc::new(RefCell::new(Arc::new(paths))), None);
    for (case, query, count) in [
        ("ascii", "PLAN", 40),
        ("unicode", "计划", 40),
        ("anchor", "note1#计划", 40),
        ("empty", "", 40),
        ("miss", "absent", 0),
    ] {
        let source = format!("中文😀 [[{query}");
        let rope = Rope::from(source.clone());
        let mut samples = Vec::new();
        let mut expected = None;
        for iteration in 0..35 {
            let start = Instant::now();
            let task = window
                .update(cx, |_, window, cx| {
                    provider.completions(
                        &rope,
                        source.len(),
                        lsp_types::CompletionContext {
                            trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                            trigger_character: None,
                        },
                        window,
                        cx,
                    )
                })
                .unwrap();
            let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
                panic!("expected completion array");
            };
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(items.len(), count, "{case}");
            let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&items).unwrap()));
            assert_eq!(expected.get_or_insert_with(|| hash.clone()), &hash);
            if iteration >= 5 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "completion_cpu={}",
            serde_json::json!({
                "case": case, "items": count, "samples": samples.len(),
                "source_sha256": format!("{:x}", Sha256::digest(source.as_bytes())),
                "result_sha256": expected.unwrap(),
                "p50_ms": samples[14], "p95_ms": samples[28], "max_ms": samples[29],
            })
        );
    }
}
