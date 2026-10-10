//! Same-host Markdown CPU stages; use generated performance corpus only.
use inkstone_core::{index, markdown, syntax::Snapshot};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

fn measure<T>(name: &str, mut run: impl FnMut() -> T, mut verify: impl FnMut(&T)) {
    let mut samples = Vec::new();
    for round in 0..25 {
        let start = Instant::now();
        let result = black_box(run());
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        verify(&result);
        if round >= 5 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "stage={name} samples=20 p50_ms={:.6} p95_ms={:.6}",
        samples[9], samples[18]
    );
}

fn main() {
    let file = std::env::args()
        .nth(1)
        .expect("usage: markdown_benchmark GENERATED_FILE");
    let source = std::fs::read_to_string(file).unwrap();
    let snapshot = Snapshot::new(&source);
    let spans = markdown::spans_snapshot(&snapshot);
    let parsed = serde_json::to_vec(&index::parse_snapshot(&snapshot)).unwrap();
    println!(
        "profile={} bytes={} source_sha256={:x} spans_sha256={:x} metadata_sha256={:x}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        source.len(),
        Sha256::digest(source.as_bytes()),
        Sha256::digest(format!("{spans:?}").as_bytes()),
        Sha256::digest(&parsed),
    );
    measure(
        "spans",
        || markdown::spans_snapshot(black_box(&snapshot)),
        |result| assert_eq!(result, &spans),
    );
    // Keep insertions inside the middle text node, so the paragraph/heading
    // remains eligible for local parsing. Use a real parser as the oracle.
    let paragraphs: Vec<_> = snapshot
        .ast
        .as_deref()
        .and_then(markdown_parser::mdast::Node::children)
        .into_iter()
        .flatten()
        .filter(|node| matches!(node, markdown_parser::mdast::Node::Paragraph(_)))
        .filter_map(markdown_parser::mdast::Node::position)
        .filter(|p| p.end.offset - p.start.offset > 8)
        .collect();
    if let Some(position) = paragraphs.get(paragraphs.len() / 2) {
        let mut offset = (position.start.offset + position.end.offset) / 2;
        while !source.is_char_boundary(offset) {
            offset += 1;
        }
        for (stage, insertion) in [("edit_plain", "中文😀"), ("edit_markup", "**中文😀**")]
        {
            let mut edited = source.clone();
            edited.insert_str(offset, insertion);
            let expected = Snapshot::new(&edited);
            println!(
                "stage={stage} edited_ast_sha256={:x}",
                Sha256::digest(format!("{:?}", expected.ast).as_bytes())
            );
            measure(
                stage,
                || {
                    snapshot
                        .update_block(black_box(&edited))
                        .expect("local edit")
                },
                |result| {
                    assert_eq!(result.ast, expected.ast);
                    assert_eq!(result.source, expected.source);
                    assert_eq!(result.structural, expected.structural);
                },
            );
        }
    }
}
