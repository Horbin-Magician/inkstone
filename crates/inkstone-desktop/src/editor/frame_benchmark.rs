//! Manual, same-host headless GUI CPU timings. Never a native input latency claim.
use super::*;
use core::prelude::v1::test;
use std::time::Instant;

/// The exact corpus-v1 long paragraph, including its Unicode graphemes. This
/// deliberately does not split the paragraph to make wrapping cheaper.
#[gpui::test]
#[ignore = "manual same-host long paragraph CPU comparison"]
fn long_paragraph_frame_performance(cx: &mut TestAppContext) {
    use sha2::{Digest, Sha256};
    cx.update(gpui_kit::init);
    let unit = "中文输入、English text、组合字符 e\u{301} 和 emoji 👩‍💻，用于检查光标与选区。";
    let source = format!("# 长段落\n\n{}\n", unit.repeat(10_000));
    let hash = format!("{:x}", Sha256::digest(source.as_bytes()));
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tools/performance/corpus-v1.json")).unwrap();
    let fixture = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "long-paragraph.md")
        .unwrap();
    assert_eq!(fixture["sha256"], hash);
    for live in [false, true] {
        let handle = cx.add_window(|w, cx| {
            let mut pane = EditorPane::new(&source, w, cx);
            pane.live = live;
            pane
        });
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        for position in [0, 5_000, 9_999] {
            let offset = "# 长段落\n\n".len() + position * unit.len();
            handle
                .update(&mut visual, |pane, _, cx| {
                    pane.editor.update(cx, |state, cx| {
                        state.set_selected_range(offset..offset, cx);
                        state.reveal_cursor(cx);
                    });
                })
                .unwrap();
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let mut samples = [vec![], vec![], vec![]];
            for i in 0..25 {
                // Invalidating the same input as a blink forces a real redraw;
                // this is CPU work, not a timer or native presentation latency.
                let start = Instant::now();
                handle
                    .update(&mut visual, |pane, _, cx| {
                        pane.editor.update(cx, |_, cx| cx.notify());
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                let idle = start.elapsed().as_secs_f64() * 1000.;
                let start = Instant::now();
                handle
                    .update(&mut visual, |pane, _, cx| {
                        pane.editor.update(cx, |state, cx| {
                            let mut offset = state.scroll_offset();
                            offset.y += px(if i % 2 == 0 { -24. } else { 24. });
                            state.set_scroll_offset(offset, cx);
                        });
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                let scroll = start.elapsed().as_secs_f64() * 1000.;
                let start = Instant::now();
                handle
                    .update(&mut visual, |pane, w, cx| {
                        pane.editor.update(cx, |state, cx| {
                            state.replace(if i % 2 == 0 { "字" } else { "\n" }, w, cx);
                        });
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                let edit = start.elapsed().as_secs_f64() * 1000.;
                if i >= 5 {
                    for (samples, value) in samples.iter_mut().zip([idle, scroll, edit]) {
                        samples.push(value);
                    }
                }
                handle
                    .update(&mut visual, |pane, w, cx| {
                        pane.editor.update(cx, |state, cx| {
                            state.undo(&gpui_component::input::Undo, w, cx);
                            assert_eq!(state.value().as_ref(), source.as_str());
                            state.set_selected_range(offset..offset, cx);
                            state.reveal_cursor(cx);
                        });
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
            }
            for (stage, values) in ["idle_redraw", "scroll", "edit"].into_iter().zip(samples) {
                let mut sorted = values.clone();
                sorted.sort_by(f64::total_cmp);
                println!(
                    "{}",
                    serde_json::json!({
                        "benchmark": "long_paragraph_frame_cpu_v1", "live": live,
                        "position": position, "bytes": source.len(), "sha256": hash,
                        "stage": stage, "samples_ms": values,
                        "p50_ms": sorted[9], "p95_ms": sorted[18], "max_ms": sorted[19],
                    })
                );
            }
        }
        handle
            .update(&mut visual, |_, w, _| w.remove_window())
            .unwrap();
    }
}

#[gpui::test]
#[ignore = "manual same-host formula performance acceptance"]
fn formula_frame_performance(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for count in [100, 1000] {
        let source = (0..count)
            .map(|i| format!("第{i}行 $\\frac{{x_{{{i}}}^2}}{{1+x_{{{i}}}}}$\n\n"))
            .collect::<String>();
        let start = Instant::now();
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        for _ in 0..12 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let open_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut typing = vec![];
        let mut scrolling = vec![];
        for i in 0..25 {
            let start = Instant::now();
            handle
                .update(&mut visual, |p, w, cx| {
                    p.editor.update(cx, |s, cx| {
                        let end = s.value().len();
                        s.set_selected_range(end..end, cx);
                        s.replace("x", w, cx);
                    });
                    p.update_presentation(cx);
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            if i >= 5 {
                typing.push(start.elapsed().as_secs_f64() * 1000.);
            }
            visual.run_until_parked();
            let start = Instant::now();
            handle
                .update(&mut visual, |p, _, cx| {
                    p.editor.update(cx, |s, cx| {
                        s.set_scroll_offset(point(px(0.), px(-((i % 15) as f32 * 40.))), cx);
                    });
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            if i >= 5 {
                scrolling.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        typing.sort_by(f64::total_cmp);
        scrolling.sort_by(f64::total_cmp);
        println!(
            "formula_frame_cpu count={count} open_settle_ms={open_ms:.3} typing_p50_ms={:.3} typing_p95_ms={:.3} scroll_p50_ms={:.3} scroll_p95_ms={:.3}",
            typing[9], typing[18], scrolling[9], scrolling[18]
        );
    }
}

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
