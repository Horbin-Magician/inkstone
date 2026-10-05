use super::*;
use crate::test_support::PlatformKeys;
use core::prelude::v1::test;
#[gpui::test]
fn plain_edits_keep_following_live_syntax_and_undo_coordinates(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source =
        "# Heading\n\n中文 ordinary paragraph. end\n\n## Later\n\n- [x] task\n\n**bold** [[note]]";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    cx.run_until_parked();
    handle
        .update(cx, |pane, window, cx| {
            let start = source.find("ordinary").unwrap();
            pane.editor.update(cx, |state, cx| {
                state.set_selected_range(start..start + 8, cx);
                state.replace("中文😀 prose", window, cx);
            });
            pane.update_presentation(cx);
            let current = pane.editor.read(cx).value();
            assert_eq!(
                pane.syntax_snapshot.as_ref().unwrap().ast,
                inkstone_core::syntax::Snapshot::new(&current).ast
            );
            assert_eq!(pane.spans, markdown::spans(&current));
            assert_eq!(pane.parsed.tasks, index::parse(&current).tasks);
            assert_eq!(pane.parsed.headings, index::parse(&current).headings);
            pane.editor.update(cx, |state, cx| {
                state.undo(&gpui_component::input::Undo, window, cx)
            });
            pane.update_presentation(cx);
            assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            assert_eq!(pane.spans, markdown::spans(source));
            assert_eq!(pane.parsed.tasks, index::parse(source).tasks);
        })
        .unwrap();
}

/// Editing and reading wrap in the same column, so a line holds the same text.
#[gpui::test]
fn editing_text_column_matches_the_reading_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "砚".repeat(400);
    let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for (width, readable, line_numbers) in [
        (900., true, false),
        (900., false, false),
        (520., true, true),
    ] {
        handle
            .update(&mut visual, |pane, window, cx| {
                pane.readable_width = readable;
                pane.reading = false;
                pane.fold_headings = false;
                pane.set_fold_options(true, false, window, cx);
                pane.editor.update(cx, |state, cx| {
                    state.set_line_number(line_numbers, window, cx)
                });
                cx.notify();
            })
            .unwrap();
        visual.simulate_resize(size(px(width), px(600.)));
        // The line-number font size is applied while drawing, so draw twice
        // and read the settled column.
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let (editing_wrap, editing_bounds) = handle
            .update(&mut visual, |pane, _, cx| {
                let editor = pane.editor.read(cx);
                (
                    editor.wrap_width().expect("the editor must lay out"),
                    editor.text_bounds().expect("the editor must lay out"),
                )
            })
            .unwrap();
        handle
            .update(&mut visual, |pane, _, cx| {
                pane.reading = true;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let (reading_wrap, reading_bounds) = handle
            .update(&mut visual, |pane, _, cx| {
                (
                    pane.preview
                        .read(cx)
                        .measured_wrap_width()
                        .expect("the reading view must lay out"),
                    pane.reading_bounds(cx),
                )
            })
            .unwrap();
        assert!(
            (editing_wrap - reading_wrap).abs() <= px(1.),
            "text wraps differently at {width}px, readable {readable}, line numbers {line_numbers}: editing {editing_wrap:?}, reading {reading_wrap:?}"
        );
        let gutter = editing_bounds.left() + (editing_bounds.size.width - editing_wrap);
        assert!(
            (gutter - reading_bounds.left()).abs() <= px(1.)
                && (gutter + editing_wrap - reading_bounds.right()).abs() <= px(1.),
            "text column is misaligned at {width}px, readable {readable}, line numbers {line_numbers}: editing text {gutter:?}..{:?}, reading {:?}",
            gutter + editing_wrap,
            reading_bounds
        );
    }
}

#[gpui::test]
fn reading_currency_preserves_dollars_links_and_footnote_numbering(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "spent $5 [link][r] [^n] and $10; formula $x^2$\n\n[r]: target.md\n[^n]: body";
    let handle = cx.add_window(|w, cx| {
        let mut p = EditorPane::new(source, w, cx);
        p.reading = true;
        p
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..5 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, _, cx| {
            p.preview.update(cx, |s, cx| {
                s.select_all(cx);
                let text = s.selected_text();
                assert!(
                    text.contains("$5 link") && text.contains("$10") && text.contains("1. body"),
                    "{text:?}"
                );
                assert!(
                    !text.contains("[link][r]") && !text.contains("[^n]"),
                    "{text:?}"
                );
            })
        })
        .unwrap();
}
#[gpui::test]
fn html_scripts_and_details_render_copy_and_fold_without_editing_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "H<sub>2</sub>O and x<sup>**2**</sup> <kbd>Ctrl</kbd>\n\n<details><summary>展开</summary><p>中文正文</p></details>\n\n<details open><summary>展开</summary><p>第二正文</p></details>";
    let handle = cx.add_window(|w, cx| {
        let mut p = EditorPane::new(source, w, cx);
        p.reading = true;
        p
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..6 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, _, cx| {
            p.preview.update(cx, |s, cx| {
                s.select_all(cx);
                let copied = s.selected_text();
                assert!(copied.contains("H2O"), "{copied}");
                assert!(copied.contains("Ctrl"));
                assert!(!copied.contains("<sup>") && !copied.contains("**2**"));
                assert_eq!(s.callout_states().len(), 0);
            });
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
    let offset = source.find("<details>").unwrap();
    let selector = Box::leak(format!("callout-fold-{offset}").into_boxed_str());
    let button = visual.debug_bounds(selector).unwrap();
    visual.simulate_click(button.center(), Modifiers::default());
    visual.run_until_parked();
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(
                p.preview.read(cx).callout_states().values().next(),
                Some(&false)
            );
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn live_edits_present_current_syntax_before_background_reading(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# PLACE\n\n**粗体** [[目标|别名]]\n\n- [x] 完成任务\n\n> 引用\n\n---\n\n正文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    for replacement in ["空 格", "，标点!", "*强调*", "中文😀", "标题\n新行", ""] {
        let immediate = handle
            .update(cx, |p, window, cx| {
                p.live = true;
                p.editor
                    .update(cx, |state, cx| state.set_value(source, window, cx));
                p.update_presentation(cx);
                p.editor.update(cx, |state, cx| {
                    state.set_selected_range(2..7, cx);
                    state.replace(replacement, window, cx);
                });
                // Do not run the executor: this is the frame immediately
                // after typing, before the reading view can finish parsing.
                p.update_presentation(cx);
                let text = p.editor.read(cx).value();
                assert_eq!(p.spans, markdown::spans(&text));
                assert_eq!(p.parsed.tasks.len(), 1);
                assert_eq!(p.live_tasks.len(), 1);
                assert!(!p.live_quotes.is_empty());
                assert_eq!(p.live_rules.len(), 1);
                (
                    p.editor.read(cx).concealed_ranges(),
                    p.decorations.get_ranges(cx).len(),
                    p.live_tasks[0].range.clone(),
                    p.live_quotes.clone(),
                    p.live_rules.clone(),
                )
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                p.update_presentation(cx);
                let settled = (
                    p.editor.read(cx).concealed_ranges(),
                    p.decorations.get_ranges(cx).len(),
                    p.live_tasks[0].range.clone(),
                    p.live_quotes.clone(),
                    p.live_rules.clone(),
                );
                assert_eq!(immediate, settled, "replacement: {replacement:?}");
            })
            .unwrap();
    }
}

#[gpui::test]
fn reading_comments_leave_no_placeholder_in_copied_text(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "pre%%隐藏%%fix\n\n%%inline%% # literal";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.reading = true;
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            assert!(p.parsed.headings.is_empty());
            p.preview.update(cx, |preview, cx| {
                preview.select_all(cx);
                let selected = preview.selected_text();
                assert!(selected.contains("prefix"), "{selected:?}");
                assert!(selected.contains("# literal"), "{selected:?}");
                assert!(!selected.contains(['\u{1}', '\u{2060}']));
                assert!(!selected.contains("<!--"));
                assert!(!selected.contains("隐藏"));
            });
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn comments_remain_editable_and_are_absent_from_reading(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "before %%中文 **not bold**%% after\n\n%%\n- [ ] hidden\n%%\nend";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| p.update_presentation(cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, window, cx| {
            p.update_presentation(cx);
            assert_eq!(
                p.spans
                    .iter()
                    .filter(|span| span.kind == Kind::Comment)
                    .count(),
                2
            );
            assert!(!p.spans.iter().any(|span| span.kind == Kind::Strong));
            assert!(p.editor.read(cx).concealed_ranges().is_empty());
            assert!(p.live_tasks.is_empty());
            assert!(!p.rendered.markdown.contains("not bold"));
            assert!(!p.rendered.markdown.contains("hidden"));
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
            p.live = false;
            p.update_presentation(cx);
            assert_eq!(p.decorations.get_ranges(cx).len(), 2);
            p.editor.update(cx, |state, cx| {
                state.set_selected_range(9..15, cx);
                state.replace("替换", window, cx);
                state.undo(&gpui_component::input::Undo, window, cx);
                assert_eq!(state.value().as_ref(), source);
            });
        })
        .unwrap();
}
#[gpui::test]
fn reading_position_restores_after_parse_and_explicit_jump_wins(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = (0..60)
        .map(|i| format!("Paragraph {i} 中文内容"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.reading = true;
            p.restore_reading_position(
                inkstone_core::preferences::ReadingPosition {
                    block: 20,
                    offset: 7.,
                },
                cx,
            );
            assert_eq!(p.reading_position(cx).block, 20);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..5 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, window, cx| {
            assert!(p.pending_reading_position.is_none());
            assert_eq!(p.reading_position(cx).block, 20);
            assert!((p.reading_position(cx).offset - 7.).abs() < 1.);
            p.restore_reading_position(
                inkstone_core::preferences::ReadingPosition {
                    block: 30,
                    offset: 0.,
                },
                cx,
            );
            p.jump(0, window, cx);
        })
        .unwrap();
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.reading_position(cx).block, 0)
        })
        .unwrap();
}
#[gpui::test]
fn committed_wiki_prefix_opens_attachment_completion(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.set_paths(Arc::new(vec!["图示.svg".into()]));
            p.editor
                .update(cx, |s, cx| s.replace_text_in_range(None, "![[图", w, cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            let state = p.editor.read(cx);
            let menu = state.completion_menu_state();
            assert!(menu.open);
            assert_eq!(menu.items.len(), 1);
            assert_eq!(menu.items[0].label, "图示.svg");
            assert!(menu.query.is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn footnote_numbers_follow_references_and_map_exact_markers(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source =
        "before[^b] middle[^a] again[^b]\n\n[^a]: Alpha\n\n[^b]: Beta\n\n    - item\n\nlast";
    let state = cx.new(|cx| TextViewState::markdown(source, cx));
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        let text = s.rendered_text();
        let text_ref = text.as_str();
        assert!(text_ref.starts_with("before1 middle2 again1"), "{text_ref}");
        assert!(text_ref.find("last").unwrap() < text_ref.find("Beta").unwrap());
        assert!(text_ref.find("Beta").unwrap() < text_ref.find("Alpha").unwrap());
        assert!(text_ref.contains("item"));
        let at = text
            .position_for_source_offset(source.find("again").unwrap() + 5)
            .unwrap();
        assert!(text_ref[at..].starts_with('1'), "{}", &text_ref[at..]);
    });
    let unused = cx.new(|cx| TextViewState::markdown("[^unused]: x[^b]\n\n[^b]: Secret", cx));
    cx.run_until_parked();
    unused.read_with(cx, |s, _| {
        assert!(!s.rendered_text().as_str().contains("Secret"))
    });
    let nested = cx.new(|cx| {
        TextViewState::markdown(
            "[^a] first\n\n[^a]: A[^c]\n\nlater[^b]\n\n[^b]: B\n\n[^c]: C",
            cx,
        )
    });
    cx.run_until_parked();
    nested.read_with(cx, |s, _| {
        let text = s.rendered_text();
        assert!(text.as_str().contains("later2"), "{}", text.as_str());
        assert!(text.as_str().contains("A3"));
    });
    let streamed = cx.new(|cx| TextViewState::markdown("body[^a]\n\n[^a]: A", cx));
    cx.run_until_parked();
    streamed.update(cx, |s, cx| s.push_str("\n\nnew[^b]\n\n[^b]: B", cx));
    cx.run_until_parked();
    streamed.read_with(cx, |s, _| {
        let text = s.rendered_text();
        assert_eq!(text.as_str().matches("body1").count(), 1);
        assert_eq!(text.as_str().matches("new2").count(), 1);
        assert!(text.as_str().find("new2").unwrap() < text.as_str().find("1. A").unwrap());
    });
}
#[gpui::test]
fn footnote_links_scroll_to_definition_and_back(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = format!(
        "[^note] Intro\n\n{}\n[^note]: X",
        "paragraph\n\n".repeat(60)
    );
    let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.reading = true;
            p.update_presentation(cx);
            p.focus_view(w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(600.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let bounds = handle
        .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
        .unwrap();
    visual.simulate_click(bounds.origin + point(px(3.), px(12.)), Modifiers::default());
    for _ in 0..5 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let footer = handle
        .update(&mut visual, |p, _, cx| {
            assert!(p.preview.read(cx).scroll_position().item_ix > 0);
            let offset = p.rendered.markdown.rfind("[^").unwrap();
            p.preview.read(cx).bounds_for_source_offset(offset).unwrap()
        })
        .unwrap();
    visual.simulate_click(
        point(footer.left() + px(40.), footer.bottom() - px(12.)),
        Modifiers::default(),
    );
    for _ in 0..5 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.preview.read(cx).scroll_position().item_ix, 0);
            assert_eq!(p.editor.read(cx).value(), source);
            assert!(p.reading);
        })
        .unwrap();
}
#[gpui::test]
fn callout_state_survives_a_new_view_instance(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> [!note]- Restore\n> - [ ] task\n\nend";
    let first = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    first
        .update(cx, |p, _, cx| {
            p.reading = true;
            cx.notify();
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(first.into(), cx);
    visual.simulate_resize(size(px(900.), px(650.)));
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let bounds = first
        .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
        .unwrap();
    visual.simulate_click(
        point(bounds.right() - px(21.), bounds.top() + px(24.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    let saved = first
        .update(&mut visual, |p, _, cx| p.callout_states(cx))
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved.values().next(), Some(&false));
    let second = visual.add_window(|w, cx| {
        let mut p = EditorPane::new(source, w, cx);
        p.reading = true;
        p.restore_callout_states(&saved, cx);
        p
    });
    let mut restored = VisualTestContext::from_window(second.into(), &visual);
    restored.simulate_resize(size(px(900.), px(650.)));
    for _ in 0..4 {
        restored.run_until_parked();
        restored.update(|w, cx| w.draw(cx).clear(cx));
    }
    let bounds = second
        .update(&mut restored, |p, _, cx| p.reading_bounds(cx))
        .unwrap();
    restored.simulate_click(
        bounds.origin + point(px(32.), px(56.)),
        Modifiers::default(),
    );
    restored.run_until_parked();
    second
        .update(&mut restored, |p, _, cx| {
            assert!(p.editor.read(cx).value().contains("- [x] task"))
        })
        .unwrap();
}

#[gpui::test]
fn callout_fold_and_task_click_preserve_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> [!warning]- **注意😀**\n> - [ ] body\n\nend";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.reading = true;
            p.update_presentation(cx);
            p.focus_view(w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(650.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let bounds = handle
        .update(&mut visual, |p, _, cx| {
            let rendered = p.preview.read(cx).rendered_text();
            assert!(rendered.as_str().contains("注意😀"));
            assert!(rendered.as_str().contains("body"));
            assert!(!rendered.as_str().contains("[!warning]"));
            p.preview.update(cx, |s, cx| {
                s.set_selection_format(gpui_base::text::SelectionFormat::Source, cx);
                s.select_all(cx);
                assert!(s.selected_text().contains("[!warning]- **注意😀**"));
                s.clear_selection(cx);
            });
            p.reading_bounds(cx)
        })
        .unwrap();
    visual.simulate_click(
        bounds.origin + point(px(32.), px(56.)),
        Modifiers::default(),
    );
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), source)
        })
        .unwrap();
    visual.simulate_click(
        point(bounds.right() - px(21.), bounds.top() + px(24.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_click(
        bounds.origin + point(px(32.), px(56.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    handle
        .update(&mut visual, |p, _, cx| {
            assert!(p.editor.read(cx).value().contains("- [x] body"))
        })
        .unwrap();
    handle
        .update(&mut visual, |p, w, cx| {
            p.reading = false;
            p.focus_view(w, cx);
            cx.notify();
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |p, w, cx| {
            p.reading = true;
            p.focus_view(w, cx);
            cx.notify();
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_click(
        bounds.origin + point(px(32.), px(56.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), source)
        })
        .unwrap();
}

#[gpui::test]
fn callout_parsing_handles_default_nested_and_lazy_content(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, required) in [
        ("> [!note]\n> body", "Note"),
        ("> [!tip] Title\nlazy body", "lazy body"),
        ("> [!note] Outer\n> > [!tip] Inner\n> > text", "Inner"),
        ("> [!danger] Hello", "Hello"),
        ("> [!note] ---", "---"),
        ("> [!note] # title", "# title"),
    ] {
        let state = cx.new(|cx| TextViewState::markdown(source, cx));
        cx.run_until_parked();
        state.read_with(cx, |s, _| {
            let rendered = s.rendered_text();
            assert!(
                rendered.as_str().contains(required),
                "{}",
                rendered.as_str()
            );
            assert!(!rendered.as_str().contains("[!"));
        });
    }
    let source = "```\n> [!note] literal\n```";
    let state = cx.new(|cx| TextViewState::markdown(source, cx));
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert!(s.rendered_text().as_str().contains("[!note]"))
    });
}

#[gpui::test]
fn callout_title_link_keeps_original_reference(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for new_tab in [false, true] {
        for source in [
            "> [!note] [go](other.md)\n> body",
            "> [!note] [go][ref]\n> body\n\n[ref]: other.md",
            "> [!note] [go][ref]\n> body\n\n[ref]: other.md\n[ref]: wrong.md",
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let received = Rc::new(RefCell::new(Vec::new()));
            let capture = received.clone();
            let _subscription = handle
                .update(cx, |p, w, cx| {
                    p.reading = true;
                    p.update_presentation(cx);
                    p.focus_view(w, cx);
                    cx.subscribe(
                        &cx.entity(),
                        move |_, _, event: &EditorEvent, _| match event {
                            EditorEvent::FollowReference(r) => {
                                capture.borrow_mut().push((r.clone(), false))
                            }
                            EditorEvent::FollowReferenceInNewTab(r) => {
                                capture.borrow_mut().push((r.clone(), true))
                            }
                            _ => (),
                        },
                    )
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.simulate_resize(size(px(900.), px(650.)));
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let bounds = handle
                .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
                .unwrap();
            visual.simulate_click(
                bounds.origin + point(px(52.), px(24.)),
                Modifiers {
                    control: new_tab && !cfg!(target_os = "macos"),
                    platform: new_tab && cfg!(target_os = "macos"),
                    ..Modifiers::default()
                },
            );
            let diagnostic = handle
                .update(&mut visual, |p, _, cx| {
                    format!(
                        "{} => {}",
                        p.rendered.markdown,
                        p.preview.read(cx).rendered_text().as_str()
                    )
                })
                .unwrap();
            assert_eq!(received.borrow().len(), 1, "{source}; {diagnostic}");
            assert_eq!(received.borrow()[0].0.target, "other.md");
            assert_eq!(received.borrow()[0].1, new_tab);
        }
    }
}
#[gpui::test]
fn reading_jump_expands_callout_body(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> [!note]- Header\n> [body](other.md)";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let received = Rc::new(std::cell::Cell::new(false));
    let capture = received.clone();
    let _subscription = handle
        .update(cx, |p, w, cx| {
            p.reading = true;
            p.update_presentation(cx);
            p.focus_view(w, cx);
            cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                if matches!(event, EditorEvent::FollowReference(_)) {
                    capture.set(true);
                }
            })
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(650.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |p, w, cx| {
            p.jump(source.find("[body]").unwrap(), w, cx)
        })
        .unwrap();
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let bounds = handle
        .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
        .unwrap();
    visual.simulate_click(
        bounds.origin + point(px(32.), px(56.)),
        Modifiers::default(),
    );
    assert!(received.get());
}
#[gpui::test]
fn table_tab_navigation_adds_rows_and_undo_preserves_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "| a | b |\n| --- | --- |\n| c | d |";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(&s.value()[s.selected_range()], "b")
    });
    visual.simulate_platform_keystrokes("shift-tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(&s.value()[s.selected_range()], "a")
    });
    let end = source.rfind('d').unwrap() + 1;
    handle
        .update(&mut visual, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(end..end, cx))
        })
        .unwrap();
    visual.simulate_platform_keystrokes("tab");
    editor.read_with(&visual, |s, _| assert!(s.value().ends_with("\n|  |  |")));
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), end..end);
    });
}
#[gpui::test]
fn folding_moves_the_caret_and_search_and_jump_reveal_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# 第一章\n正文\n## 子标题\n隐藏目标😀\n# 第二章\n末尾";
    let second = source.find("# 第二章").unwrap();
    let hidden = source.find("隐藏").unwrap();
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(hidden..hidden, cx));
            p.update_presentation(cx);
            p.editor.clone()
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let before = editor.read_with(&visual, |s, _| {
        s.range_to_bounds(&(second..second)).unwrap().top()
    });
    handle
        .update(&mut visual, |p, w, cx| p.fold_sections(Some(true), w, cx))
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    editor.read_with(&visual, |s, _| {
        assert!(!s.folded_ranges().is_empty());
        assert!(s.selected_range().start < hidden);
        assert!(s.range_to_bounds(&(second..second)).unwrap().top() < before);
        assert_eq!(s.value(), source);
    });
    handle
        .update(&mut visual, |p, _, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_search_query("隐藏目标", false, cx);
                assert!(s.next_search_match(cx).is_some());
                assert!(
                    s.folded_ranges()
                        .iter()
                        .all(|f| !(f.start_line < 3 && 3 < f.end_line))
                );
            });
        })
        .unwrap();
    handle
        .update(&mut visual, |p, w, cx| {
            p.fold_sections(Some(true), w, cx);
            p.jump(hidden, w, cx);
            assert_eq!(p.editor.read(cx).selected_range(), hidden..hidden);
            assert!(
                p.editor
                    .read(cx)
                    .folded_ranges()
                    .iter()
                    .all(|f| !(f.start_line < 3 && 3 < f.end_line))
            );
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-a ctrl-c");
    visual.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
    handle
        .update(&mut visual, |p, w, cx| {
            p.fold_sections(Some(true), w, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(0..0, cx);
                s.replace("\n", w, cx);
            });
            p.update_presentation(cx);
        })
        .unwrap();
    visual.run_until_parked();
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), format!("\n{source}"));
        assert!(s.folded_ranges().iter().any(|f| f.start_line == 5));
    });
}

#[gpui::test]
fn folding_gutter_uses_heading_heights_and_does_not_edit_text(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# 大标题\nbody\n## 次标题\nhidden\n# 结尾\nlast";
    let target = source.find("##").unwrap();
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(target..target, cx));
            p.update_presentation(cx);
            p.editor.clone()
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let row = editor.read_with(&visual, |s, _| {
        s.range_to_bounds(&(target..target)).unwrap()
    });
    visual.simulate_click(
        point(row.left() - px(20.), row.top() + row.size.height.half()),
        Modifiers::default(),
    );
    editor.read_with(&visual, |s, _| {
        assert!(s.folded_ranges().iter().any(|f| f.start_line == 2));
        assert_eq!(s.value(), source);
    });
}
#[gpui::test]
fn triple_backticks_insert_a_closing_fence_and_preserve_context(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            for (prefix, continuation) in [
                ("", ""),
                ("  ", "  "),
                ("> ", "> "),
                ("- ", "  "),
                ("> 12. ", ">     "),
                ("- [ ] ", "  "),
            ] {
                p.editor.update(cx, |s, cx| {
                    s.set_value(prefix, w, cx);
                    s.set_selected_range(prefix.len()..prefix.len(), cx);
                    for _ in 0..3 {
                        s.replace_text_in_range(None, "`", w, cx);
                    }
                    let expected = format!("{prefix}```\n{continuation}```");
                    assert_eq!(s.value(), expected);
                    assert_eq!(s.selected_range(), prefix.len() + 3..prefix.len() + 3);
                    let end = expected.len() - 3;
                    s.set_selected_range(end..end, cx);
                    s.replace_text_in_range(None, "`", w, cx);
                    assert_eq!(s.value(), format!("{expected}`"));
                    assert_eq!(s.selected_range(), end + 1..end + 1);
                });
            }
            p.editor.update(cx, |s, cx| {
                s.set_value("before\r\n", w, cx);
                s.set_selected_range(8..8, cx);
                for _ in 0..3 {
                    s.replace_text_in_range(None, "`", w, cx);
                }
                assert_eq!(s.value(), "before\r\n```\r\n```");
                s.focus(w, cx);
            });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_platform_keystrokes("ctrl-z");
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), "before\r\n``")
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-y");
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), "before\r\n```\r\n```");
            assert_eq!(p.editor.read(cx).selected_range(), 11..11);
        })
        .unwrap();
}

#[gpui::test]
fn synchronized_text_keeps_selection_on_grapheme_boundaries(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("a", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(1..1, cx);
                assert!(s.apply_synced_text(
                    "a\u{301}",
                    &[(1..1, "\u{301}".into())],
                    gpui_base::input::SyncedHistory::Ignore,
                    false,
                    w,
                    cx
                ));
                assert_eq!(s.selected_range(), 0..0);
                assert!(!s.apply_synced_text(
                    "wrong",
                    &[(1..2, "bad".into())],
                    gpui_base::input::SyncedHistory::Ignore,
                    false,
                    w,
                    cx
                ));
                assert_eq!(s.value(), "a\u{301}");
            });
        })
        .unwrap();
}

#[gpui::test]
fn large_counts_are_deferred_and_old_requests_do_not_replace_new_selection(
    cx: &mut TestAppContext,
) {
    use inkstone_core::word_count::Counts;
    cx.update(gpui_kit::init);
    let long = "word ".repeat(3000);
    let handle = cx.add_window(|w, cx| EditorPane::new(&long, w, cx));
    handle
        .update(cx, |p, w, cx| {
            assert_eq!(p.text_counts(cx), Counts::default());
            p.editor.update(cx, |s, cx| s.set_value("short", w, cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 5
                }
            );
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
    handle
        .update(cx, |p, w, cx| {
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 5
                }
            );
            p.editor
                .update(cx, |s, cx| s.set_value(long.clone(), w, cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 5
                }
            );
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 3000,
                    characters: 15000
                }
            );
            p.editor
                .update(cx, |s, cx| s.set_selected_range(0..10000, cx));
            p.text_counts(cx);
            p.editor.update(cx, |s, cx| s.set_selected_range(0..4, cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 4
                }
            );
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 4
                }
            )
        })
        .unwrap();
}

#[gpui::test]
fn text_counts_track_selection_reading_mode_and_source_changes(cx: &mut TestAppContext) {
    use inkstone_core::word_count::Counts;
    cx.update(gpui_kit::init);
    let source = "---\ntitle: Secret\n---\n你好 world😀";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 3,
                    characters: 10
                }
            );
            let emoji = source.find('😀').unwrap();
            p.editor
                .update(cx, |s, cx| s.set_selected_range(emoji..source.len(), cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 0,
                    characters: 2
                }
            );
            let secret = source.find("Secret").unwrap();
            p.editor
                .update(cx, |s, cx| s.set_selected_range(secret..secret + 6, cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 1,
                    characters: 6
                }
            );
            p.reading = true;
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 3,
                    characters: 10
                }
            );
            p.editor
                .update(cx, |s, cx| s.set_value(format!("{source} ok"), w, cx));
            assert_eq!(
                p.text_counts(cx),
                Counts {
                    words: 4,
                    characters: 13
                }
            );
        })
        .unwrap();
}

#[gpui::test]
fn multiple_empty_ordered_items_exit_and_undo_as_one_change(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "7. \n8. \n9. text";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(3..3, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "\n\n7. text");
        assert!(s.has_multiple_selections());
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert!(s.has_multiple_selections());
    });
}

#[gpui::test]
fn multiple_carets_continue_numbered_lists_and_undo_together(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "8. a\n9. b";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(4..4, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down enter");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), "8. a\n9. \n10. b\n11. ");
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), source);
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    visual.simulate_input("中");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "8. a\n9. 中\n10. b\n11. 中")
    });
}

#[gpui::test]
fn multiple_carets_soft_break_together_and_restore_on_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "- 中文\n- 中文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(8..8, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down shift-enter");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), "- 中文\n  \n- 中文\n  ");
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), source);
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    visual.simulate_input("x");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- 中文\n  x\n- 中文\n  x")
    });
}

#[gpui::test]
fn shift_enter_replaces_multiline_selection_with_one_undo_step(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "    中文\r\n    tail";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(4..source.len(), cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("shift-enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "    \r\n");
        assert_eq!(s.selected_range(), 6..6);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), 4..source.len());
    });
}

#[gpui::test]
fn home_and_shift_home_respect_markdown_prefixes_and_soft_wrap(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> - [!] 中文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.live = false;
            p.set_auto_pairing(false, false, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("home");
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 8..8));
    visual.simulate_platform_keystrokes("home");
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 0..0));
    visual.simulate_platform_keystrokes("end shift-home");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.selected_range(), 8..source.len())
    });
    visual.simulate_platform_keystrokes("shift-home");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.selected_range(), 0..source.len())
    });
    let long = format!("> - [!] {}", "长文本".repeat(80));
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.set_value(long.clone(), w, cx);
                s.set_selected_range(long.len()..long.len(), cx);
            })
        })
        .unwrap();
    visual.simulate_resize(size(px(300.), px(400.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("home");
    editor.read_with(&visual, |s, _| assert!(s.selected_range().start > 8));
    visual.simulate_platform_keystrokes("home");
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 8..8));
    visual.simulate_platform_keystrokes("home");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.selected_range(), 0..0);
        assert_eq!(s.value(), long);
    });
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.set_value("> - [!] a\n> - [!] b", w, cx);
                s.set_selected_range(9..9, cx);
            })
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down home");
    visual.simulate_input("X");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), "> - [!] Xa\n> - [!] Xb");
    });
}

#[gpui::test]
fn custom_tasks_render_as_checkboxes_and_click_preserves_source_mapping(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "- [✓] **中文**\n- [!] second";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx)
            });
            p.reading = true;
            p.update_presentation(cx);
            p.focus_view(w, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(700.), px(400.)));
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let bounds = handle
        .update(&mut visual, |p, _, cx| {
            let text = p.preview.read(cx).rendered_text();
            assert!(text.as_str().contains("中文"));
            assert!(!text.as_str().contains("[✓]"));
            assert!(!text.as_str().contains("[!]"));
            assert_eq!(
                text.position_for_source_offset(source.find("中文").unwrap()),
                text.as_str().find("中文")
            );
            p.preview.update(cx, |s, cx| {
                s.set_selection_format(gpui_base::text::SelectionFormat::Source, cx);
                s.select_all(cx);
                assert_eq!(s.selected_text(), source);
                s.clear_selection(cx);
            });
            p.preview.read(cx).bounds_for_source_offset(0).unwrap()
        })
        .unwrap();
    visual.simulate_click(bounds.origin + point(px(7.), px(12.)), Modifiers::default());
    handle
        .update(&mut visual, |p, w, cx| {
            assert_eq!(p.editor.read(cx).value(), "- [ ] **中文**\n- [!] second");
            assert_eq!(
                p.editor.read(cx).selected_range(),
                source.len() - 2..source.len() - 2
            );
            p.editor
                .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
            assert_eq!(p.editor.read(cx).value(), source);
        })
        .unwrap();
}

#[gpui::test]
fn enter_after_soft_list_continuation_resumes_marker_and_undoes_once(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("- 中文", w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(8..8, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("shift-enter");
    visual.simulate_input("续行");
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- 中文\n  续行\n- ");
        assert_eq!(s.selected_range(), 20..20);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n  续行"));
}

#[gpui::test]
fn reading_soft_breaks_follow_strict_setting_without_changing_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "第一行\r\n第二行\n\nhard  \nbreak";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.reading = true;
            p.update_presentation(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(700.), px(400.)));
    let mut soft_height = None;
    for strict in [false, true, false] {
        handle
            .update(&mut visual, |p, _, cx| {
                p.strict_line_breaks = strict;
                cx.notify();
            })
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, _, cx| {
                let preview = p.preview.read(cx);
                let text = preview.rendered_text();
                assert!(
                    text.as_str().contains(if strict {
                        "第一行 第二行"
                    } else {
                        "第一行\n第二行"
                    }),
                    "{}",
                    text.as_str()
                );
                assert!(text.as_str().contains("hard\nbreak"));
                let first = preview.bounds_for_source_offset(0).unwrap();
                assert_eq!(
                    text.position_for_source_offset(source.find("第二行").unwrap()),
                    text.as_str().find("第二行")
                );
                if strict {
                    assert!(first.size.height < soft_height.unwrap());
                } else {
                    soft_height = Some(first.size.height);
                }
                assert_eq!(p.editor.read(cx).value(), source);
            })
            .unwrap();
    }
}

#[gpui::test]
fn fold_options_remove_disabled_ranges_and_keep_other_folds(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# Heading\ntext\n- parent\n  - child\n```\ncode\n```";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.fold_sections(Some(true), w, cx);
            assert_eq!(
                p.editor
                    .read(cx)
                    .folded_ranges()
                    .iter()
                    .map(|f| f.start_line)
                    .collect::<Vec<_>>(),
                vec![0, 2, 4]
            );
            p.set_fold_options(false, true, w, cx);
            assert_eq!(
                p.editor
                    .read(cx)
                    .folded_ranges()
                    .iter()
                    .map(|f| f.start_line)
                    .collect::<Vec<_>>(),
                vec![2, 4]
            );
            p.set_fold_options(false, false, w, cx);
            assert!(p.editor.read(cx).folded_ranges().is_empty());
            p.fold_sections(Some(true), w, cx);
            assert!(p.editor.read(cx).folded_ranges().is_empty());
            p.set_fold_options(true, false, w, cx);
            p.fold_sections(Some(true), w, cx);
            assert_eq!(
                p.editor
                    .read(cx)
                    .folded_ranges()
                    .iter()
                    .map(|f| f.start_line)
                    .collect::<Vec<_>>(),
                vec![0, 4]
            );
            assert_eq!(p.editor.read(cx).value(), source);
        })
        .unwrap();
}

#[gpui::test]
fn multiple_carets_indent_quote_lines_once_and_undo_together(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> 中文\n> 中文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(5..5, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down tab");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), "> \t中文\n> \t中文");
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), source);
    });
    visual.simulate_platform_keystrokes("ctrl-y shift-tab");
    editor.read_with(&visual, |s, _| {
        assert!(s.has_multiple_selections());
        assert_eq!(s.value(), source);
    });
    editor.update(&mut visual, |s, cx| s.set_selected_range(2..2, cx));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let at = editor.read_with(&visual, |s, _| s.range_to_bounds(&(5..5)).unwrap().origin);
    visual.simulate_click(
        at + point(px(1.), px(8.)),
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );
    editor.read_with(&visual, |s, _| assert!(s.has_multiple_selections()));
    visual.simulate_platform_keystrokes("tab");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "> \t中文\n> 中文"));
}

#[gpui::test]
fn tab_indents_the_line_from_the_middle_of_plain_text(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> 中文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(5..5, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "> \t中文");
        assert_eq!(s.selected_range(), 6..6);
    });
    visual.simulate_platform_keystrokes("shift-tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), 5..5);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "> \t中文"));
}

#[gpui::test]
fn exiting_an_ordered_item_restores_numbers_and_caret_on_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "1. 中文\n2. \n3. following";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(13..13, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "1. 中文\n\n2. following");
        assert_eq!(s.selected_range(), 10..10);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), 13..13);
    });
}

#[gpui::test]
fn ordered_list_enter_and_renumber_are_one_undoable_edit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "9. 中文\n10. following\n11. end";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(9..9, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "9. 中文\n10. \n11. following\n12. end");
        assert_eq!(s.selected_range(), 14..14);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), 9..9);
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "9. 中文\n10. \n11. following\n12. end")
    });
}

#[gpui::test]
fn smart_lists_toggle_enter_but_keep_tab_and_soft_continuation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "- 中文";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("shift-enter");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n  "));
    visual.simulate_platform_keystrokes("ctrl-z");
    handle
        .update(&mut visual, |p, _, _| p.smart_lists = false)
        .unwrap();
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n"));
    visual.simulate_platform_keystrokes("ctrl-z tab");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "\t- 中文"));
}

#[gpui::test]
fn occurrence_matching_normalizes_unicode_and_restores_original_bytes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, selection, keys, expected) in [
        ("é e\u{301} é", 0..2, "ctrl-shift-l", "x x x"),
        ("é e\u{301} é", 0..2, "ctrl-d ctrl-d", "x x é"),
        ("ff ﬀ ＦＦ", 0..2, "ctrl-shift-l", "x x ＦＦ"),
        ("f ﬀ", 0..1, "ctrl-shift-l", "x x"),
        ("① 1 ①", 0..3, "ctrl-shift-l", "x x x"),
        ("😀 é e\u{301}", 5..7, "ctrl-shift-l", "😀 x x"),
        ("e\u{301} é", 0..3, "ctrl-d", "x x"),
        ("aaaaaa", 1..3, "ctrl-d", "axxa"),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(selection.clone(), cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes(keys);
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections(), "{source}");
                    assert_eq!(s.value(), source);
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), expected);
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), selection);
        });
    }
}

#[gpui::test]
fn select_all_occurrences_keeps_adjacent_ranges_and_undoes_once(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, selection, expected) in [
        ("cat scatter cat Cat", 12..15, "x sxter x Cat"),
        ("中文中文", 0..6, "xx"),
        ("e\u{301}e\u{301}", 0..3, "xx"),
        ("😀😀", 0..4, "xx"),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(selection.clone(), cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-shift-l");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    assert_eq!(s.selected_range(), selection);
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), expected);
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), selection);
            assert!(s.has_multiple_selections());
        });
    }
}

#[gpui::test]
fn select_all_occurrences_respects_empty_selection_and_reference_limit(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                for count in [1001, 1002] {
                    let source = "a ".repeat(count);
                    s.set_value(&source, w, cx);
                    s.set_selected_range(0..0, cx);
                    s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, w, cx);
                    assert_eq!(s.selected_range(), 0..0);
                    s.set_selected_range(0..1, cx);
                    s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, w, cx);
                    assert_eq!(s.has_multiple_selections(), count == 1001);
                    assert_eq!(s.value(), source);
                }
            });
        })
        .unwrap();
}

#[gpui::test]
fn ctrl_d_selects_words_wraps_and_edits_all_occurrences(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "cat scatter cat Cat cat";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(13..13, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-d");
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 12..15));
    visual.simulate_platform_keystrokes("ctrl-d ctrl-d ctrl-d");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                assert!(s.has_multiple_selections());
                s.replace_text_in_range(None, "犬", w, cx);
                assert_eq!(s.value(), "犬 scatter 犬 Cat 犬");
            });
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert!(s.has_multiple_selections());
    });
}

#[gpui::test]
fn ctrl_d_preserves_unicode_words_and_explicit_substrings(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, selection, expected) in [
        ("中文 中文", 0..0, "x x"),
        ("e\u{301} e\u{301}", 0..0, "x x"),
        ("foobar barfoo", 3..6, "foox xfoo"),
        ("😀 😀", 0..4, "x x"),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(selection, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-d ctrl-d");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), expected);
                });
            })
            .unwrap();
    }
}

#[gpui::test]
fn selected_code_after_double_backticks_gets_fence_line_breaks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, range, expected, selected) in [
        ("``中文``", 2..8, "```\n中文\n```", 4..10),
        ("``\n中文\n``", 2..10, "```\n中文\n```", 3..11),
        (
            "> ``中文``\r\n",
            4..10,
            "> ```\r\n> 中文\r\n> ```\r\n",
            9..15,
        ),
        ("``\r\n中文\r\n``", 2..12, "```\r\n中文\r\n```", 3..13),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(range.clone(), cx);
                    s.focus(w, cx);
                    s.replace_text_in_range(None, "`", w, cx);
                    assert_eq!(s.value(), expected);
                    assert_eq!(s.selected_range(), selected);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), range);
        });
        visual.simulate_platform_keystrokes("ctrl-y");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), expected);
            assert_eq!(s.selected_range(), selected);
        });
    }
}

#[gpui::test]
fn multiple_selected_code_ranges_become_fences_and_keep_selection(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "``中``\n``文``";
    let expected = "```\n中\n```\n```\n文\n```";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(2..2, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down shift-right");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.replace_text_in_range(None, "`", w, cx);
                assert_eq!(s.value(), expected);
                assert!(s.has_multiple_selections());
                s.replace_text_in_range(None, "x", w, cx);
                assert_eq!(s.value(), "```\nx\n```\n```\nx\n```");
            });
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), expected));
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
}

#[gpui::test]
fn multiple_carets_pair_symmetric_markers_and_code_fences(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, offset, typed, paired, inserted) in [
        ("\n", 0, "*", "**\n**", "*x*\n*x*"),
        ("\n", 0, "_", "__\n__", "_x_\n_x_"),
        ("\n", 0, "'", "''\n''", "'x'\n'x'"),
        ("\n", 0, "\"", "\"\"\n\"\"", "\"x\"\n\"x\""),
        ("\n", 0, "`", "``\n``", "`x`\n`x`"),
        (
            "``\n``",
            2,
            "`",
            "```\n```\n```\n```",
            "```x\n```\n```x\n```",
        ),
        (
            "> ``\r\n> ``",
            4,
            "`",
            "> ```\r\n> ```\r\n> ```\r\n> ```",
            "> ```x\r\n> ```\r\n> ```x\r\n> ```",
        ),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(offset..offset, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-alt-down");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, typed, w, cx);
                    assert_eq!(s.value(), paired);
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), inserted);
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), paired));
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert!(s.has_multiple_selections());
        });
    }
}

#[gpui::test]
fn symmetric_pairing_can_skip_and_insert_in_one_batch(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("\n", w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(0..0, cx);
                s.replace_text_in_range(None, "*", w, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down");
    for _ in 0..2 {
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "*", w, cx);
                    assert_eq!(s.value(), "**\n**");
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), "**x\n*x*");
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "**\n"));
    }
}

#[gpui::test]
fn multiple_carets_insert_and_skip_pairs_without_extra_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("\n", w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(0..0, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-alt-down");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.replace_text_in_range(None, "(", w, cx);
                assert_eq!(s.value(), "()\n()");
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "()\n()");
                s.replace_text_in_range(None, "x", w, cx);
                assert_eq!(s.value(), "()x\n()x");
            });
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "()\n()"));
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "\n");
        assert!(s.has_multiple_selections());
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "()\n()");
            });
        })
        .unwrap();
}

#[gpui::test]
fn mixed_carets_and_selections_pair_only_when_every_position_allows_it(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, select, enabled, typed, expected) in [
        ("中文\n", true, true, "(", "(中文)\n()"),
        ("\nx", false, true, "(", "(\n(x"),
        ("\n", false, false, "(", "(\n("),
        ("中文\n", true, true, "*", "*中文*\n**"),
        ("中文\n", true, true, "\"", "\"中文\"\n\"\""),
        ("x\n", false, true, "*", "*x\n*"),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(enabled, enabled, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(0..0, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-alt-down");
        if select {
            visual.simulate_platform_keystrokes("shift-end");
        }
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, typed, w, cx);
                    assert_eq!(s.value(), expected);
                    assert!(s.has_multiple_selections());
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
    }
}

#[gpui::test]
fn multiple_selections_surround_preserve_direction_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for reversed in [false, true] {
        let source = "中文\n中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    let offset = if reversed { 6 } else { 0 };
                    s.set_selected_range(offset..offset, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes(if reversed {
            "ctrl-alt-down shift-home"
        } else {
            "ctrl-alt-down shift-end"
        });
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "*", w, cx);
                    assert_eq!(s.value(), "*中文*\n*中文*");
                    s.replace_text_in_range(None, "(", w, cx);
                    assert_eq!(s.value(), "*(中文)*\n*(中文)*");
                });
            })
            .unwrap();
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "*中文*\n*中文*"));
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
        visual.simulate_platform_keystrokes("ctrl-y");
        if reversed {
            visual.simulate_platform_keystrokes("shift-right");
        }
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(
                        s.value(),
                        if reversed {
                            "*中x*\n*中x*"
                        } else {
                            "*x*\n*x*"
                        }
                    );
                });
            })
            .unwrap();
    }
}

#[gpui::test]
fn multiline_selection_pairing_distinguishes_inline_markers_and_code(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("中\n文", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                for (typed, expected) in [("(", "()"), ("*", "**"), ("`", "`中\n文`")] {
                    s.set_value("中\n文", w, cx);
                    s.set_selected_range(0..7, cx);
                    s.replace_text_in_range(None, typed, w, cx);
                    assert_eq!(s.value(), expected);
                }
            });
        })
        .unwrap();
}

#[gpui::test]
fn multiple_carets_delete_pairs_together_and_restore_carets_on_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (source, offset, enabled, select, deleted, inserted) in [
        ("中()\n文[]", 4, true, false, "中\n文", "中(x)\n文[x]"),
        ("()\nab", 1, true, false, ")\nb", "(x)\naxb"),
        ("()\n[]", 1, false, false, ")\n]", "(x)\n[x]"),
        ("**\n__", 1, true, false, "\n", "*x*\n_x_"),
        ("()\n[]", 1, true, true, ")\n]", "x)\nx]"),
        ("\\()\n ()", 2, true, false, "\\)\n )", "\\(x)\n (x)"),
    ] {
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(enabled, enabled, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(offset..offset, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_platform_keystrokes("ctrl-alt-down");
        if select {
            visual.simulate_platform_keystrokes("shift-left");
        }
        visual.simulate_platform_keystrokes("backspace");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), deleted, "{source}");
            assert!(s.has_multiple_selections());
        });
        visual.simulate_platform_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
        visual.simulate_platform_keystrokes("ctrl-y");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), deleted));
        visual.simulate_platform_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), inserted);
                });
            })
            .unwrap();
    }
}

#[gpui::test]
fn pair_tracking_expires_when_keyboard_navigation_leaves_the_line(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("\nnext", w, cx));
    let editor = handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(0..0, cx);
                s.replace_text_in_range(None, "(", w, cx);
                s.focus(w, cx);
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("down up");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                assert_eq!(s.selected_range(), 1..1);
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "())\nnext");
                s.set_value("", w, cx);
                s.replace_text_in_range(None, "(", w, cx);
            });
        })
        .unwrap();
    visual.simulate_platform_keystrokes("left right");
    handle
        .update(&mut visual, |_, w, cx| {
            editor.update(cx, |s, cx| {
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "()");
                assert_eq!(s.selected_range(), 2..2);
            })
        })
        .unwrap();
}

#[gpui::test]
fn paired_closers_skip_once_but_existing_text_is_never_swallowed(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                for closer in [")", "]", "}", "*", "_", "`", "\""] {
                    s.set_value(closer, w, cx);
                    s.set_selected_range(0..0, cx);
                    s.replace_text_in_range(None, closer, w, cx);
                    assert_eq!(s.value(), closer.repeat(2));
                }
                s.set_value("", w, cx);
                s.replace_text_in_range(None, "(", w, cx);
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "()");
                assert_eq!(s.selected_range(), 2..2);
                s.set_selected_range(1..1, cx);
                s.replace_text_in_range(None, ")", w, cx);
                assert_eq!(s.value(), "())");
                s.set_value("中文😀", w, cx);
                s.set_selected_range(0..10, cx);
                s.replace_text_in_range(None, "*", w, cx);
                s.set_selected_range(11..11, cx);
                s.replace_text_in_range(None, "*", w, cx);
                assert_eq!(s.value(), "*中文😀*");
                assert_eq!(s.selected_range(), 12..12);
            });
        })
        .unwrap();
}

#[gpui::test]
fn automatic_pairs_are_independent_and_respect_escape_words_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    handle
        .update(cx, |p, w, cx| {
            for (brackets, markdown) in [(true, true), (true, false), (false, true), (false, false)]
            {
                p.set_auto_pairing(brackets, markdown, cx);
                for (typed, paired, enabled) in [
                    ("(", "()", brackets),
                    ("[", "[]", brackets),
                    ("\"", "\"\"", brackets),
                    ("*", "**", markdown),
                    ("_", "__", markdown),
                    ("`", "``", markdown),
                ] {
                    p.editor.update(cx, |s, cx| {
                        s.set_value("", w, cx);
                        s.replace_text_in_range(None, typed, w, cx);
                        assert_eq!(s.value(), if enabled { paired } else { typed });
                        assert_eq!(s.selected_range(), 1..1);
                    });
                }
            }
            p.set_auto_pairing(true, true, cx);
            p.editor.update(cx, |s, cx| {
                s.set_value("中文😀", w, cx);
                s.set_selected_range(0..10, cx);
                s.replace_text_in_range(None, "*", w, cx);
                assert_eq!(s.value(), "*中文😀*");
                assert_eq!(s.selected_range(), 1..11);
            });
            for source in ["word", "\\"] {
                p.editor.update(cx, |s, cx| {
                    s.set_value(source, w, cx);
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.replace_text_in_range(None, "*", w, cx);
                    assert_eq!(s.value(), format!("{source}*"));
                });
            }
            let other = cx.new(|cx| EditorPane::new("", w, cx));
            other.update(cx, |other, cx| other.set_auto_pairing(false, false, cx));
            p.editor.update(cx, |s, cx| {
                s.set_value("", w, cx);
                s.replace_text_in_range(None, "(", w, cx);
                assert_eq!(s.value(), "()");
                s.focus(w, cx);
            });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_platform_keystrokes("backspace");
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), "")
        })
        .unwrap();
    visual.simulate_platform_keystrokes("ctrl-z");
    handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).value(), "()")
        })
        .unwrap();
}

#[gpui::test]
fn markdown_keys_continue_indent_and_undo_as_single_edits(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "- [x] 中文👩‍💻";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx)
            });
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
    });
    visual.simulate_platform_keystrokes("tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- [x] 中文👩‍💻\n\t- [ ] ")
    });
    visual.simulate_platform_keystrokes("shift-tab");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
    });
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- [x] 中文👩‍💻\n"));
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ");
        assert_eq!(s.selected_range(), s.value().len()..s.value().len());
    });
    visual.simulate_platform_keystrokes("ctrl-z ctrl-z ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), source.len()..source.len());
    });
    visual.simulate_platform_keystrokes("ctrl-y");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
    });
}

#[gpui::test]
fn configured_tabs_apply_to_plain_text_and_markdown_lists(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for (width, hard_tabs) in [(2, false), (6, false), (4, true)] {
        for source in ["plain", "- 中文😀"] {
            let editor = handle
                .update(&mut visual, |p, w, cx| {
                    p.indentation = gpui_base::input::TabSize {
                        tab_size: width,
                        hard_tabs,
                    };
                    p.editor.update(cx, |s, cx| {
                        s.set_tab_size(p.indentation, cx);
                        s.set_value(source, w, cx);
                        s.set_selected_range(0..0, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_platform_keystrokes("tab");
            let indent = if hard_tabs {
                "\t".into()
            } else {
                " ".repeat(width)
            };
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), format!("{indent}{source}"))
            });
            visual.simulate_platform_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
        }
    }
}

#[gpui::test]
fn markdown_keys_respect_literal_blocks_completion_and_composition(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "```\n- code\n```";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(10..10, cx));
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("enter");
    editor.read_with(&visual, |s, _| assert_eq!(s.value(), "```\n- code\n\n```"));
    handle
        .update(&mut visual, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_value("- 中文", w, cx);
                s.set_selected_range(8..8, cx);
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
            });
            let before = p.editor.read(cx).value();
            p.markdown_key(inkstone_core::markdown_edit::Key::Enter, w, cx);
            assert_eq!(p.editor.read(cx).value(), before);
            p.editor.update(cx, |s, cx| {
                s.unmark_text(w, cx);
                s.present_completion_items(
                    2,
                    "",
                    vec![lsp_types::CompletionItem {
                        label: "候选".into(),
                        ..Default::default()
                    }],
                    cx,
                );
            });
            p.markdown_key(inkstone_core::markdown_edit::Key::Enter, w, cx);
            assert_eq!(p.editor.read(cx).value(), before);
        })
        .unwrap();
}

#[gpui::test]
fn markdown_prefix_backspace_restores_caret_on_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> - [ ] 中文👩‍💻";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.live = false;
            p.editor.update(cx, |s, cx| s.set_selected_range(8..8, cx));
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("backspace");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), "> 中文👩‍💻");
        assert_eq!(s.selected_range(), 2..2);
    });
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), source);
        assert_eq!(s.selected_range(), 8..8);
    });
    visual.simulate_platform_keystrokes("ctrl-end shift-enter");
    editor.read_with(&visual, |s, _| {
        assert_eq!(s.value(), format!("{source}\n>       "))
    });
}
#[gpui::test]
fn wrapped_headings_reflow_and_keep_the_following_caret_visible(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = format!("# {}\nbody", "长标题😀".repeat(35));
    let end = source.len();
    let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
    let state = handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(end..end, cx));
            p.update_presentation(cx);
            p.editor.clone()
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for width in [700., 280., 900., 330.] {
        visual.simulate_resize(size(px(width), px(320.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        state.read_with(&visual, |s, _| {
            assert_eq!(s.selected_range(), end..end);
            assert_eq!(s.value().as_ref(), source);
            let (mut caret, _) = s.cursor_layout().unwrap();
            caret.origin += s.scroll_offset();
            assert!(
                s.input_bounds().intersects(&caret),
                "width={width}, caret={caret:?}, viewport={:?}",
                s.input_bounds()
            );
        });
    }
}
#[gpui::test]
fn ime_candidate_stays_on_composing_line_before_redraw(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for source in [
        "paragraph\n\n# Heading\nbody",
        "- item\n- \n# Heading\nbody",
    ] {
        let offset = source.find("\n# Heading").unwrap();
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(offset..offset, cx));
                p.update_presentation(cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1000.), px(650.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    let anchor = s.range_to_bounds(&(offset..offset)).unwrap();
                    // macOS can request geometry synchronously after setMarkedText,
                    // before another frame has shaped the new preedit text.
                    for text in ["n", "ni h", "ni hao", "你"] {
                        let end = offset + text.encode_utf16().count();
                        s.replace_and_mark_text_in_range(None, text, None, w, cx);
                        for index in offset..=end {
                            let candidate = s
                                .bounds_for_range(index..index, s.input_bounds(), w, cx)
                                .unwrap();
                            assert_eq!(
                                candidate.top(),
                                anchor.top(),
                                "preedit={text}, index={index}"
                            );
                            assert_eq!(candidate.size.height, anchor.size.height);
                        }
                    }
                });
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    let caret = offset + "你".len();
                    let expected = s.range_to_bounds(&(caret..caret)).unwrap();
                    let candidate = s
                        .bounds_for_range(offset + 1..offset + 1, s.input_bounds(), w, cx)
                        .unwrap();
                    assert_eq!(candidate, expected);
                });
            })
            .unwrap();
    }
}

#[gpui::test]
fn live_heading_font_height_hit_testing_and_ime_agree(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# Heading\nbody\n## Sub\nlast";
    let body = source.find("body").unwrap();
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let editor = handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx)
            });
            p.update_presentation(cx);
            p.editor.clone()
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1000.), px(650.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let (header, paragraph) = editor.read_with(&visual, |s, _| {
        (
            s.range_to_bounds(&(0..0)).unwrap(),
            s.range_to_bounds(&(body..body)).unwrap(),
        )
    });
    assert!(header.size.height > paragraph.size.height);
    assert!((header.bottom() - paragraph.top()).abs() < px(0.1));
    visual.simulate_click(
        paragraph.origin + point(px(1.), px(8.)),
        Modifiers::default(),
    );
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), body..body));
    visual.simulate_platform_keystrokes("up");
    editor.read_with(&visual, |s, _| assert!(s.selected_range().start < body));
    visual.simulate_platform_keystrokes("down");
    editor.read_with(&visual, |s, _| assert_eq!(s.selected_range().start, body));
    handle
        .update(&mut visual, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                assert_eq!(
                    s.character_index_for_point(paragraph.origin + point(px(1.), px(8.)), w, cx),
                    Some(body)
                );
            })
        })
        .unwrap();
    handle
        .update(&mut visual, |p, _, cx| {
            p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
            p.update_presentation(cx);
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                let source_bounds = s.range_to_bounds(&(2..2)).unwrap();
                let viewport = s.input_bounds();
                let candidate = s.bounds_for_range(2..2, viewport, w, cx).unwrap();
                assert_eq!(candidate.size.height, source_bounds.size.height);
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
            })
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    editor.read_with(&visual, |s, _| {
        assert!(s.range_to_bounds(&(2..2)).unwrap().size.height > paragraph.size.height)
    });
}

struct ConcealProbe {
    state: Entity<EditorState>,
}
impl Render for ConcealProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            Editor::new(&self.state)
                .appearance(false)
                .bordered(false)
                .h_full()
                .text_size(px(16.)),
        )
    }
}
#[gpui::test]
fn tab_display_width_updates_geometry_wrapping_and_mouse_selection(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "\t中文😀\n \tsecond\n**bold**\tend\né👩‍💻\tend";
    let handle = cx.add_window(|w, cx| {
        let state = cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source)
                .line_number(false)
        });
        state.update(cx, |s, cx| {
            s.set_tab_size(
                gpui_base::input::TabSize {
                    tab_size: 2,
                    hard_tabs: true,
                },
                cx,
            );
            s.focus(w, cx);
        });
        ConcealProbe { state }
    });
    let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(700.), px(300.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let small = state.read_with(&visual, |s, _| s.range_to_bounds(&(0..1)).unwrap());
    assert!(small.size.width > px(0.));
    state.update(&mut visual, |s, cx| {
        s.set_tab_size(
            gpui_base::input::TabSize {
                tab_size: 8,
                hard_tabs: true,
            },
            cx,
        );
        let start = source.find("**bold").unwrap();
        s.set_concealed_ranges(vec![start..start + 2, start + 6..start + 8], cx);
    });
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let large = state.read_with(&visual, |s, _| {
        let tab = s.range_to_bounds(&(0..1)).unwrap();
        assert!((f32::from(tab.size.width) - f32::from(small.size.width) * 4.).abs() < 0.1);
        let space = source.find(" \t").unwrap();
        let after_space = s.range_to_bounds(&(space + 1..space + 2)).unwrap();
        assert!(
            (f32::from(after_space.size.width) - f32::from(tab.size.width) * 7. / 8.).abs() < 0.1
        );
        let marker = source.find("**bold").unwrap();
        let unicode_tab = source.rfind('\t').unwrap();
        let unicode_width = s
            .range_to_bounds(&(unicode_tab..unicode_tab + 1))
            .unwrap()
            .size
            .width;
        assert!((f32::from(unicode_width) - f32::from(tab.size.width) * 6. / 8.).abs() < 0.1);
        assert_eq!(
            s.range_to_bounds(&(marker..marker + 2)).unwrap().size.width,
            px(0.)
        );
        tab
    });
    visual.simulate_click(
        large.origin + point(large.size.width - px(1.), px(8.)),
        Modifiers::default(),
    );
    state.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 1..1));
    visual.simulate_resize(size(px(75.), px(300.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    state.read_with(&visual, |s, _| {
        let before = s.range_to_bounds(&(0..0)).unwrap();
        let after = s.range_to_bounds(&(7..7)).unwrap();
        assert!(after.origin.y > before.origin.y);
        assert_eq!(s.value().as_ref(), source);
    });
    visual.simulate_platform_keystrokes("ctrl-a ctrl-c");
    visual.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
}

#[gpui::test]
fn concealment_removes_marker_width_without_changing_source_or_copy(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "**中文😀** [[很长的路径/笔记|别名]]";
    let spans = markdown::spans(source);
    let masks: Vec<_> = spans.iter().flat_map(|s| s.markers.clone()).collect();
    let handle = cx.add_window(|w, cx| {
        let state = cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source)
                .line_number(false)
        });
        state.update(cx, |s, cx| {
            s.set_concealed_ranges(masks, cx);
            s.focus(w, cx);
        });
        ConcealProbe { state }
    });
    let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(300.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    state.read_with(&visual, |s, _| {
        assert_eq!(s.value().as_ref(), source);
        for span in &spans {
            for marker in &span.markers {
                assert_eq!(s.range_to_bounds(marker).unwrap().size.width, px(0.));
            }
            assert!(s.range_to_bounds(&span.content).unwrap().size.width > px(0.));
        }
    });
    let origin = state.read_with(&visual, |s, _| s.range_to_bounds(&(2..2)).unwrap().origin);
    visual.simulate_click(origin + point(px(1.), px(8.)), Modifiers::default());
    state.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 2..2));
    visual.simulate_platform_keystrokes("ctrl-a ctrl-c");
    visual.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
}
#[gpui::test]
fn concealed_prefix_does_not_create_a_blank_wrapped_row(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "**👨‍👩‍👧‍👦**";
    let handle = cx.add_window(|w, cx| {
        let state = cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source)
                .line_number(false)
        });
        state.update(cx, |s, cx| {
            s.set_concealed_ranges(vec![0..2, source.len() - 2..source.len()], cx);
            s.focus(w, cx);
        });
        ConcealProbe { state }
    });
    let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(25.), px(300.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    state.read_with(&visual, |s, _| {
        let prefix = s.range_to_bounds(&(0..0)).unwrap();
        let content = s.range_to_bounds(&(2..2)).unwrap();
        assert_eq!(
            prefix.origin.y, content.origin.y,
            "a concealed marker must not become a blank visual row"
        );
        assert_eq!(s.value().as_ref(), source);
    });
}
#[gpui::test]
fn setext_marker_line_disappears_and_reveals_for_selection_and_search(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "标题\r\n===\r\n正文\r\n# Next\r\nend";
    let body = source.find("正文").unwrap();
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(body..body, cx));
            p.update_presentation(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let hidden_y = handle
        .update(&mut visual, |p, _, cx| {
            assert_eq!(p.editor.read(cx).concealed_lines(), &[1]);
            assert!(p.editor.read(cx).folded_ranges().is_empty());
            let y = p
                .editor
                .read(cx)
                .range_to_bounds(&(body..body))
                .unwrap()
                .origin
                .y;
            p.editor.update(cx, |s, cx| s.set_selected_range(0..0, cx));
            p.update_presentation(cx);
            y
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |p, window, cx| {
            assert!(p.editor.read(cx).concealed_lines().is_empty());
            let shown_y = p
                .editor
                .read(cx)
                .range_to_bounds(&(body..body))
                .unwrap()
                .origin
                .y;
            assert!(shown_y > hidden_y + px(10.));
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx);
                s.set_search_query("===", false, cx);
            });
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_lines().is_empty());
            p.editor.update(cx, |s, cx| s.close_search(cx));
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).concealed_lines(), &[1]);
            p.editor.update(cx, |s, cx| {
                s.restore_fold_lines(&[0], cx);
                s.undo(&gpui_component::input::Undo, window, cx);
                assert_eq!(s.value().as_ref(), source);
                assert_eq!(s.folded_ranges().len(), 1);
                s.restore_fold_lines(&[], cx);
                let occurrence = source.find('e').unwrap();
                s.set_selected_range(occurrence..occurrence + 1, cx);
                s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, window, cx);
                assert!(s.has_multiple_selections());
                assert!(s.apply_selection_transform(
                    |text, _| Some((text.to_string(), vec![body..body, 0..0])),
                    window,
                    cx,
                ));
                assert_eq!(s.selected_range(), body..body);
            });
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_lines().is_empty());
            p.live = false;
            p.editor
                .update(cx, |s, cx| s.set_selected_range(body..body, cx));
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_lines().is_empty());
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn multiline_setext_content_uses_uniform_heading_metrics(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "MMMM\r\nMMMM\r\n===\r\n\r\nMMMM";
    let second = source.find('\n').unwrap() + 1;
    let body = source.rfind("MMMM").unwrap();
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(body..body, cx));
            p.update_presentation(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    handle
        .update(&mut visual, |p, _, cx| {
            let state = p.editor.read(cx);
            let first = state.range_to_bounds(&(0..4)).unwrap();
            let next = state.range_to_bounds(&(second..second + 4)).unwrap();
            let paragraph = state.range_to_bounds(&(body..body + 4)).unwrap();
            assert_eq!(first.size.width, next.size.width);
            assert_eq!(first.size.height, next.size.height);
            assert!(next.size.width > paragraph.size.width * 1.5);
            assert_eq!(state.concealed_lines(), &[2]);
            assert_eq!(state.value().as_ref(), source);
            p.live = false;
            p.update_presentation(cx);
        })
        .unwrap();
    visual.update(|w, cx| w.draw(cx).clear(cx));
    handle
        .update(&mut visual, |p, _, cx| {
            let state = p.editor.read(cx);
            let first = state.range_to_bounds(&(0..4)).unwrap();
            let next = state.range_to_bounds(&(second..second + 4)).unwrap();
            let paragraph = state.range_to_bounds(&(body..body + 4)).unwrap();
            assert_eq!(first.size, next.size);
            assert_eq!(next.size, paragraph.size);
            assert!(state.concealed_lines().is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn concealment_rejects_partial_graphemes_and_does_not_enter_history(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "**👩‍💻e\u{301}**\r\n尾行";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_concealed_ranges(
                    vec![
                        0..2,
                        2..6,
                        source.find('\r').unwrap()..source.find('\r').unwrap() + 2,
                    ],
                    cx,
                );
                assert_eq!(s.concealed_ranges(), vec![0..2]);
                s.undo(&gpui_component::input::Undo, w, cx);
                assert_eq!(s.value().as_ref(), source);
                s.set_selected_range(source.len()..source.len(), cx);
                s.replace("新", w, cx);
                s.undo(&gpui_component::input::Undo, w, cx);
                assert_eq!(s.value().as_ref(), source);
            });
        })
        .unwrap();
}
#[gpui::test]
fn concealed_whole_line_removes_all_wraps_and_clears_after_edits(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = format!("Title\n{}\n尾行", "=".repeat(300));
    let end = source.find("尾行").unwrap();
    let handle = cx.add_window(|w, cx| {
        let state = cx.new(|cx| {
            EditorState::new(w, cx)
                .default_value(source.clone())
                .line_number(false)
        });
        state.update(cx, |s, cx| {
            s.set_concealed_lines(vec![0, 6, 6, 7, source.len()], cx);
            assert_eq!(s.concealed_lines(), &[1]);
            s.focus(w, cx);
        });
        ConcealProbe { state }
    });
    let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for width in [200., 100., 300.] {
        visual.simulate_resize(size(px(width), px(300.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        state.read_with(&visual, |s, _| {
            let first = s.range_to_bounds(&(0..0)).unwrap();
            let last = s.range_to_bounds(&(end..end)).unwrap();
            assert_eq!(last.origin.y - first.origin.y, s.line_height().unwrap());
            assert!(s.folded_ranges().is_empty());
            assert_eq!(s.value().as_ref(), source);
        });
    }
    state.update_in(&mut visual, |s, window, cx| {
        s.set_selected_range(0..0, cx);
        s.replace("前缀\n", window, cx);
        assert!(s.concealed_lines().is_empty());
        s.undo(&gpui_component::input::Undo, window, cx);
        assert_eq!(s.value().as_ref(), source);
        assert!(s.concealed_lines().is_empty());
    });
}
#[gpui::test]
fn live_markers_reveal_for_the_caret_and_for_syntax_search(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "**粗体**\n其他行";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx)
            });
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).concealed_ranges().len(), 2);
            p.editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_ranges().is_empty());
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx);
                s.set_search_query("**", false, cx);
            });
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_ranges().is_empty());
            p.editor.update(cx, |s, cx| s.close_search(cx));
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).concealed_ranges().len(), 2);
            p.live = false;
            p.update_presentation(cx);
            assert!(p.editor.read(cx).concealed_ranges().is_empty());
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn ime_keeps_other_lines_compact_and_commits_only_source_text(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "**Title**\n文本";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_selected_range(source.len()..source.len(), cx)
            });
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, w, cx| {
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).concealed_ranges(), vec![0..2, 7..9]);
            p.editor.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
            });
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).concealed_ranges(), vec![0..2, 7..9]);
            p.editor.update(cx, |s, cx| {
                assert_eq!(s.marked_text_range(w, cx), Some(12..14));
                s.replace_text_in_range(None, "你", w, cx);
            });
            p.update_presentation(cx);
            assert_eq!(p.editor.read(cx).value().as_ref(), "**Title**\n文本你");
            p.editor
                .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn edits_that_change_regional_indicator_boundaries_drop_invalid_masks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "🇦🇧🇨🇩";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, w, cx| {
            p.editor.update(cx, |s, cx| {
                s.set_concealed_ranges(std::iter::once(8..16).collect(), cx);
                assert_eq!(s.concealed_ranges().len(), 1);
                s.set_selected_range(0..0, cx);
                s.replace("🇪", w, cx);
                assert!(s.concealed_ranges().is_empty());
                assert_eq!(s.value().as_ref(), "🇪🇦🇧🇨🇩");
            })
        })
        .unwrap();
}
#[gpui::test]
fn reading_block_anchor_reveals_the_correct_list_item(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "# 标题\n\n- 第一项\n- 第二项 ^second\n";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.reading = true;
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, _, cx| {
            let original = index::anchor_range(source, &p.parsed, "^second")
                .unwrap()
                .start;
            let mapped = p.rendered.output_offset(&p.current_path, original).unwrap();
            let rendered = p.preview.read(cx).rendered_text();
            let position = rendered.position_for_source_offset(mapped).unwrap();
            assert!(
                rendered.as_str()[position..].starts_with("第二项"),
                "{} at {position}",
                rendered.as_str()
            );
            assert!(!rendered.as_str().contains("^second"));
        })
        .unwrap();
}
#[gpui::test]
fn reading_link_click_emits_the_original_source_context(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("[跳转](../目的.md)", w, cx));
    let received = Rc::new(RefCell::new(Vec::new()));
    let capture = received.clone();
    let (preview, _subscription) = handle
        .update(cx, |p, w, cx| {
            p.reading = true;
            p.readable_width = false;
            p.set_reference_context("folder/note.md".into(), PathBuf::new(), Arc::default(), cx);
            p.update_presentation(cx);
            p.focus_view(w, cx);
            (
                p.preview.clone(),
                cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                    if let EditorEvent::FollowReference(reference) = event {
                        capture.borrow_mut().push(reference.clone());
                    }
                }),
            )
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(600.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    let bounds = preview.read_with(&visual, |p, _| p.bounds());
    visual.simulate_click(bounds.origin + point(px(8.), px(8.)), Modifiers::default());
    visual.simulate_mouse_down(
        bounds.origin + point(px(8.), px(8.)),
        MouseButton::Right,
        Modifiers::default(),
    );
    visual.simulate_mouse_up(
        bounds.origin + point(px(8.), px(8.)),
        MouseButton::Right,
        Modifiers::default(),
    );
    assert_eq!(received.borrow().len(), 1, "preview bounds: {bounds:?}");
    assert_eq!(received.borrow()[0].from, PathBuf::from("folder/note.md"));
    assert_eq!(received.borrow()[0].target, "../目的.md");
}
#[gpui::test]
fn reading_undo_action_uses_the_document_history(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([KeyBinding::new(
            &crate::test_support::keys("ctrl-z"),
            gpui_component::input::Undo,
            None,
        )]);
    });
    let source = "- [ ] 可以撤销\n";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| p.update_presentation(cx))
        .unwrap();
    cx.run_until_parked();
    let editor = handle
        .update(cx, |p, w, cx| {
            p.reading = true;
            p.toggle_task(p.rendered.tasks[0].clone(), true, w, cx);
            p.focus_view(w, cx);
            p.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(600.)));
    visual.update(|w, cx| w.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-z");
    editor.read_with(&visual, |e, _| assert_eq!(e.value().as_ref(), source));
}
#[gpui::test]
fn reading_task_toggle_preserves_source_mode_cursor_and_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "---\ntags: [测试]\n---\n[[长目标|短名]]\n\n- [ ] 中文😀\n";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.reading = true;
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, w, cx| {
            let target = p.rendered.tasks[0].clone();
            assert_eq!(&source[target.marker.clone()], " ");
            p.toggle_task(target, true, w, cx);
            assert!(p.reading);
            assert_eq!(p.editor.read(cx).selected_range(), 0..0);
            assert_eq!(
                p.editor.read(cx).value().as_ref(),
                source.replace("[ ]", "[x]")
            );
            p.editor
                .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
}
#[gpui::test]
fn stale_reading_task_does_not_toggle_another_item(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|w, cx| EditorPane::new("- [ ] 第一项\n", w, cx));
    handle
        .update(cx, |p, _, cx| p.update_presentation(cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, w, cx| {
            let stale = p.rendered.tasks[0].clone();
            let newer = "- [ ] 插入的新项\n- [ ] 第一项\n";
            p.editor.update(cx, |s, cx| s.set_value(newer, w, cx));
            p.toggle_task(stale, true, w, cx);
            assert_eq!(p.editor.read(cx).value().as_ref(), newer);
        })
        .unwrap();
}

#[gpui::test]
fn ctrl_hover_click_emits_wiki_target_without_changing_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for (live, shift, new_tab) in [
        (true, false, true),
        (false, false, false),
        (false, true, true),
    ] {
        let source = "中文😀 [[目录/笔记|标签]]";
        let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
        let targets = Rc::new(RefCell::new(Vec::new()));
        let captured = targets.clone();
        let _subscription = handle
            .update(cx, |pane, _, cx| {
                pane.live = live;
                pane.editor.update(cx, |state, cx| {
                    let offset = "中文😀 [[目".len();
                    state.set_selected_range(offset..offset, cx);
                });
                cx.subscribe(
                    &cx.entity(),
                    move |_, _, event: &EditorEvent, _| match event {
                        EditorEvent::FollowLink(target) => {
                            captured.borrow_mut().push((target.clone(), false))
                        }
                        EditorEvent::FollowLinkInNewTab(target) => {
                            captured.borrow_mut().push((target.clone(), true))
                        }
                        _ => (),
                    },
                )
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let position = handle
            .update(&mut visual, |pane, _, cx| {
                let state = pane.editor.read(cx);
                let (caret, _) = state.cursor_layout().unwrap();
                caret.origin + state.scroll_offset() + point(px(1.), px(5.))
            })
            .unwrap();
        let modifiers = Modifiers {
            control: !cfg!(target_os = "macos"),
            platform: cfg!(target_os = "macos"),
            shift,
            ..Default::default()
        };
        visual.simulate_mouse_move(position, None, modifiers);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_click(position, modifiers);
        assert_eq!(
            targets.borrow().as_slice(),
            &[("目录/笔记".to_string(), new_tab)]
        );
        handle
            .update(&mut visual, |pane, _, cx| {
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
}

#[gpui::test]
fn resize_keeps_visible_caret_and_selection_direction(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "中文 English 👩‍💻 e\u{301}".repeat(500);
    let handle = cx.add_window(|window, cx| EditorPane::new(&source, window, cx));
    let editor = handle.update(cx, |pane, _, _| pane.editor.clone()).unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1100.), px(800.)));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("ctrl-end shift-left");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let before = editor.read_with(&visual, |state, _| (state.selected_range(), state.cursor()));
    for width in [1900., 900.] {
        visual.simulate_resize(size(px(width), px(800.)));
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        editor.read_with(&visual, |state, _| {
            assert_eq!((state.selected_range(), state.cursor()), before);
            let (mut caret, _) = state.cursor_layout().expect("caret should be laid out");
            caret.origin += state.scroll_offset();
            assert!(state.input_bounds().intersects(&caret),
                "caret should remain visible at width {width}: caret={caret:?}, input={:?}, scroll={:?}",state.input_bounds(),state.scroll_offset());
            assert_eq!(state.value().as_ref(), source);
        });
    }
}

#[gpui::test]
fn keyboard_selection_and_delete_preserve_whole_graphemes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "A👩‍💻e\u{301}";
    let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
    let editor = handle
        .update(cx, |pane, _, cx| {
            pane.editor.update(cx, |state, cx| {
                state.set_selected_range(source.len()..source.len(), cx)
            });
            pane.editor.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_platform_keystrokes("shift-left");
    editor.read_with(&visual, |state, _| {
        assert_eq!(state.selected_range(), 12..15)
    });
    visual.simulate_platform_keystrokes("shift-left");
    editor.read_with(&visual, |state, _| {
        assert_eq!(state.selected_range(), 1..15)
    });
    visual.simulate_platform_keystrokes("backspace");
    editor.read_with(&visual, |state, _| assert_eq!(state.value().as_ref(), "A"));
    visual.simulate_platform_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    editor.read_with(&visual, |state, _| {
        assert_eq!(state.value().as_ref(), source)
    });
}

#[gpui::test]
fn newer_parse_wins_and_reading_keeps_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|window, cx| EditorPane::new("# 旧文档\n[[旧链接]]", window, cx));
    handle
        .update(cx, |pane, window, cx| {
            pane.update_presentation(cx);
            pane.editor.update(cx, |state, cx| {
                state.set_value(
                    "# 新文档😀\n\n|列|值|\n|-|-|\n|中文|数据|\n\n[[新链接]]",
                    window,
                    cx,
                )
            });
            pane.update_presentation(cx);
            pane.reading = true;
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |pane, _, cx| {
            assert_eq!(pane.parsed.headings[0].title, "新文档😀");
            assert_eq!(pane.parsed.links[0].target, "新链接");
            assert!(pane.editor.read(cx).value().contains("|中文|数据|"));
            assert!(pane.editor.read(cx).value().contains("[[新链接]]"));
        })
        .unwrap();
}

#[gpui::test]
fn component_ime_utf16_commit_cancel_and_source_preservation(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let handle = cx.add_window(|window, cx| EditorPane::new("**中😀**\n", window, cx));
    handle
        .update(cx, |pane, window, cx| {
            let editor = pane.editor.clone();
            editor.update(cx, |state, cx| {
                state.set_selected_range(12..12, cx);
                state.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                assert_eq!(state.value().as_ref(), "**中😀**\nni");
                assert_eq!(state.marked_text_range(window, cx), Some(8..10));
                state.replace_text_in_range(None, "你", window, cx);
                assert_eq!(state.value().as_ref(), "**中😀**\n你");
                assert_eq!(state.marked_text_range(window, cx), None);
                state.replace_and_mark_text_in_range(None, "hao", Some(3..3), window, cx);
                state.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
                state.unmark_text(window, cx);
                assert_eq!(state.value().as_ref(), "**中😀**\n你");
                let mut actual = None;
                assert_eq!(
                    state
                        .text_for_range(3..5, &mut actual, window, cx)
                        .as_deref(),
                    Some("😀")
                );
                assert_eq!(actual, Some(3..5));
            });
            pane.update_presentation(cx);
            assert_eq!(pane.editor.read(cx).value().as_ref(), "**中😀**\n你");
        })
        .unwrap();
}

#[gpui::test]
fn ime_rebases_markdown_overlays_through_preedit_commit_and_cancel(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "文本😀

* bullet

> quote

---

- [ ] task
";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    let insertion = "文本😀".len();
    handle
        .update(cx, |p, _, cx| {
            p.editor
                .update(cx, |s, cx| s.set_selected_range(insertion..insertion, cx));
            p.update_presentation(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    for _ in 0..4 {
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    let bullet = source.find('*').unwrap();
    let quote = source.find('>').unwrap();
    let rule = source.find("---").unwrap();
    let task = source.find("- [ ]").unwrap();
    let original_bounds: Vec<_> = [
        format!("live-list-{bullet}"),
        format!("live-quote-{quote}"),
        format!("live-rule-{rule}"),
        format!("live-task-{}", task + 3),
    ]
    .iter()
    .map(|id| visual.debug_bounds(id.clone().leak()).unwrap())
    .collect();
    for preedit in ["n", "ni", "你", "你好😀", ""] {
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, preedit, None, w, cx)
                });
                p.update_presentation(cx);
                let shift = preedit.len();
                assert_eq!(p.live_lists, vec![bullet + shift..bullet + shift + 1]);
                assert_eq!(p.live_rules, vec![rule + shift..rule + shift + 3]);
                assert_eq!(p.live_quotes[0].start, quote + shift);
                assert_eq!(p.live_tasks[0].range, task + shift..task + shift + 5);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        for (id, expected) in [
            format!("live-list-{}", bullet + preedit.len()),
            format!("live-quote-{}", quote + preedit.len()),
            format!("live-rule-{}", rule + preedit.len()),
            format!("live-task-{}", task + 3),
        ]
        .iter()
        .zip(&original_bounds)
        {
            assert_eq!(
                visual.debug_bounds(id.clone().leak()).unwrap(),
                *expected,
                "{id}: {preedit}"
            );
        }
    }
    handle
        .update(&mut visual, |p, w, cx| {
            assert_eq!(p.editor.read(cx).value().as_ref(), source);
            p.editor.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "ni", None, w, cx);
                s.replace_text_in_range(None, "你", w, cx);
            });
            p.update_presentation(cx);
            assert_eq!(p.live_lists[0].start, bullet + "你".len());
            assert_eq!(p.live_tasks[0].range.start, task + "你".len());
        })
        .unwrap();
}

#[gpui::test]
fn ime_inside_quote_rebases_border_after_batched_updates(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "> 引用正文\n\n* 后续列表\n";
    let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
    handle
        .update(cx, |p, _, cx| {
            p.editor.update(cx, |s, cx| s.set_selected_range(8..8, cx));
            p.update_presentation(cx);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |p, w, cx| {
            p.update_presentation(cx);
            let quote = p.live_quotes[0].clone();
            let bullet = p.live_lists[0].clone();
            p.editor.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "n", None, w, cx);
                s.replace_and_mark_text_in_range(None, "ni", None, w, cx);
                s.replace_and_mark_text_in_range(None, "你好😀", None, w, cx);
            });
            p.update_presentation(cx);
            let shift = "你好😀".len();
            assert_eq!(p.live_quotes[0], quote.start..quote.end + shift);
            assert_eq!(p.live_lists[0], bullet.start + shift..bullet.end + shift);
            p.editor.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "n", None, w, cx)
            });
            p.update_presentation(cx);
            assert_eq!(p.live_quotes[0], quote.start..quote.end + 1);
            assert_eq!(p.live_lists[0], bullet.start + 1..bullet.end + 1);
        })
        .unwrap();
}
