//! Manual, same-host headless GUI CPU timings. Never a native input latency claim.
use super::*;
use core::prelude::v1::test;
use std::time::Instant;

#[gpui::test]
#[ignore = "manual same-host frame performance acceptance"]
fn markdown_frame_performance(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let long = format!("# Long\n\n{}", "中文 English 😀 paragraph. ".repeat(10_000));
    let complex = format!("# Rich\n\n{}\nend", "$\\frac{1}{2}$\n\n| A | B |\n| --- | --- |\n| a | b |\n\n> [!note]+ title\n> body\n\n```mermaid\nflowchart LR\nA --> B\n```\n\n".repeat(8));
    for (name, source) in [
        ("plain", "# Small\n\n**bold** text with 中文😀\n".repeat(15)),
        ("long", long),
        ("rich", complex),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        for _ in 0..12 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let mut typing = vec![];
        let mut editing = vec![];
        let mut presentation = vec![];
        let mut drawing = vec![];
        let mut scrolling = vec![];
        for i in 0..65 {
            let start = Instant::now();
            let mut edit_ms = 0.;
            let mut presentation_ms = 0.;
            handle
                .update(&mut visual, |p, w, cx| {
                    let edit_start = Instant::now();
                    p.editor.update(cx, |s, cx| {
                        let end = s.value().len();
                        s.set_selected_range(end..end, cx);
                        s.replace("x", w, cx);
                    });
                    edit_ms = edit_start.elapsed().as_secs_f64() * 1000.;
                    let presentation_start = Instant::now();
                    p.update_presentation(cx);
                    presentation_ms = presentation_start.elapsed().as_secs_f64() * 1000.;
                })
                .unwrap();
            let draw_start = Instant::now();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            if i >= 5 {
                typing.push(start.elapsed().as_secs_f64() * 1000.);
                editing.push(edit_ms);
                presentation.push(presentation_ms);
                drawing.push(draw_start.elapsed().as_secs_f64() * 1000.);
            }
            visual.run_until_parked();
            let start = Instant::now();
            handle
                .update(&mut visual, |p, _, cx| {
                    p.editor.update(cx, |s, cx| {
                        s.set_scroll_offset(point(px(0.), px(-((i % 15) as f32 * 40.))), cx)
                    })
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            if i >= 5 {
                scrolling.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        typing.sort_by(f64::total_cmp);
        scrolling.sort_by(f64::total_cmp);
        editing.sort_by(f64::total_cmp);
        presentation.sort_by(f64::total_cmp);
        drawing.sort_by(f64::total_cmp);
        println!(
            "markdown_frame_cpu case={name} bytes={} typing_p95_ms={:.3} scroll_p95_ms={:.3}",
            source.len(),
            typing[56],
            scrolling[56]
        );
        println!(
            "markdown_frame_stages case={name} edit_p95_ms={:.3} presentation_p95_ms={:.3} draw_p95_ms={:.3}",
            editing[56], presentation[56], drawing[56]
        );
    }
}
