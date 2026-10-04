//! Public editor extension interfaces only; no component patches.
use gpui::{App, SharedString, Task, WeakEntity, Window};
use gpui_component::input::{CompletionProvider, DefinitionProvider, EditorState, Rope, RopeExt};
use inkstone_core::index::ParsedNote;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub type ParsedCache = Rc<RefCell<(SharedString, ParsedNote)>>;
#[derive(Clone, Debug, Default)]
pub struct CompletionPath {
    pub label: String,
    pub target: String,
    pub markdown: bool,
    pub alias: Option<String>,
    pub anchors: Arc<Vec<(String, String)>>,
    pub fragment: Option<String>,
}
impl From<&str> for CompletionPath {
    fn from(value: &str) -> Self {
        Self {
            label: value.into(),
            target: value.into(),
            markdown: false,
            alias: None,
            ..Default::default()
        }
    }
}
pub type PathCache = Rc<RefCell<Arc<Vec<CompletionPath>>>>;
pub struct WikiDefinitions(pub ParsedCache);
pub struct WikiCompletions(pub PathCache, pub Option<WeakEntity<EditorState>>);

fn accepts_wiki_completion(source: &str, start: usize) -> bool {
    if source[..start]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
    {
        return false;
    }
    fn blocked(node: &markdown_parser::mdast::Node, at: usize) -> bool {
        use markdown_parser::mdast::Node;
        if node
            .position()
            .is_some_and(|p| at < p.start.offset || at >= p.end.offset)
        {
            return false;
        }
        if matches!(node, Node::Code(_) | Node::InlineCode(_) | Node::Html(_)) {
            return true;
        }
        node.children()
            .is_some_and(|children| children.iter().any(|child| blocked(child, at)))
    }
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    markdown_parser::to_mdast(source, &options).is_ok_and(|root| !blocked(&root, start))
}

pub fn target_uri(target: &str) -> lsp_types::Uri {
    let hex: String = target
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("inkstone:{hex}").parse().expect("hex URI")
}
pub fn decode_uri(uri: &str) -> Option<String> {
    let hex = uri.strip_prefix("inkstone:")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<_>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}
impl DefinitionProvider for WikiDefinitions {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<lsp_types::LocationLink>>> {
        let cache = self.0.borrow();
        if cache.0.len() != text.len() || cache.0.as_ref() != text {
            return Task::ready(Ok(vec![]));
        }
        if let Some((range, id)) =
            cache.1.footnote_references.iter().find(|(range, _)| {
                range.start.saturating_sub(2) <= offset && offset <= range.end + 1
            })
            && cache
                .1
                .footnote_definitions
                .iter()
                .any(|(_, name)| name == id)
        {
            return Task::ready(Ok(vec![lsp_types::LocationLink {
                origin_selection_range: Some(lsp_types::Range::new(
                    text.offset_to_position(range.start - 2),
                    text.offset_to_position(range.end + 1),
                )),
                target_uri: format!("inkstone-footnote:{}", range.start)
                    .parse()
                    .expect("numeric URI"),
                target_range: Default::default(),
                target_selection_range: Default::default(),
            }]));
        }
        let result = cache
            .1
            .links
            .iter()
            .find(|link| link.range.contains(&offset))
            .map(|link| lsp_types::LocationLink {
                origin_selection_range: Some(lsp_types::Range::new(
                    text.offset_to_position(link.range.start),
                    text.offset_to_position(link.range.end),
                )),
                target_uri: target_uri(&link.target),
                target_range: Default::default(),
                target_selection_range: Default::default(),
            })
            .into_iter()
            .collect();
        Task::ready(Ok(result))
    }
}
impl CompletionProvider for WikiCompletions {
    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty() && !new_text.contains(['\n', '\r', ']'])
    }
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: lsp_types::CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<lsp_types::CompletionResponse>> {
        let text = text.clone();
        let snapshot = text.clone();
        let editor = self.1.clone();
        let paths = self.0.borrow().clone();
        let task = cx.background_executor().spawn(async move {
            let source = text.to_string();
            let Some(before) = source.get(..offset) else {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            };
            let Some(start) = before.rfind("[[") else {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            };
            if !accepts_wiki_completion(&source, start) {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            }
            let typed = &before[start + 2..];
            if typed.contains([']', '\n', '\r', '|']) {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            }
            // GPUI may have auto-closed only the first '['. Absorb either one
            // or two immediate closers before installing the complete wiki link.
            let end = offset
                + source[offset..]
                    .bytes()
                    .take(2)
                    .take_while(|b| *b == b']')
                    .count();
            let query = typed.to_lowercase();
            let anchor_candidates;
            let candidates = if let Some((file, fragment)) = typed.split_once('#') {
                let blocks = fragment.starts_with('^');
                let query = fragment.trim_start_matches('^').to_lowercase();
                let file = file.trim_start_matches('/');
                let mut selected = vec![];
                if file.is_empty() {
                    let markdown = paths
                        .iter()
                        .find(|p| p.alias.is_none())
                        .is_some_and(|p| p.markdown);
                    selected.push(CompletionPath {
                        markdown,
                        anchors: completion_anchors(&source),
                        ..Default::default()
                    });
                } else {
                    selected.extend(
                        paths
                            .iter()
                            .filter(|p| {
                                p.alias.is_none()
                                    && (p.label.eq_ignore_ascii_case(file)
                                        || p.target
                                            .trim_start_matches('/')
                                            .trim_end_matches(".md")
                                            .eq_ignore_ascii_case(file.trim_end_matches(".md")))
                            })
                            .cloned(),
                    );
                }
                anchor_candidates = selected
                    .iter()
                    .flat_map(|path| {
                        path.anchors
                            .iter()
                            .filter(|(label, anchor)| {
                                anchor.starts_with('^') == blocks
                                    && (label.to_lowercase().contains(&query)
                                        || anchor.to_lowercase().contains(&query))
                            })
                            .map(|(label, anchor)| {
                                let mut result = path.clone();
                                result.label = label.clone();
                                result.fragment = Some(anchor.clone());
                                if !result.markdown && anchor.contains(['|', '[', ']']) {
                                    result.markdown = true;
                                    if !result.target.is_empty() {
                                        if result.target.contains('/')
                                            && !result.target.starts_with('.')
                                            && !result.target.starts_with('/')
                                        {
                                            result.target.insert(0, '/');
                                        }
                                        result.target.push_str(".md");
                                    }
                                }
                                result
                            })
                    })
                    .take(40)
                    .collect::<Vec<_>>();
                &anchor_candidates
            } else {
                paths.as_ref()
            };
            let items = candidates
                .iter()
                .filter(|path| {
                    path.fragment.is_some()
                        || path.label.to_lowercase().contains(&query)
                        || path.target.to_lowercase().contains(&query)
                        || path
                            .alias
                            .as_ref()
                            .is_some_and(|alias| alias.to_lowercase().contains(&query))
                })
                .take(40)
                .map(|path| {
                    let label = if path.fragment.is_some() {
                        path.label.clone()
                    } else {
                        path.alias.clone().unwrap_or_else(|| {
                            path.label
                                .rsplit('/')
                                .next()
                                .unwrap_or(&path.label)
                                .to_string()
                        })
                    };
                    let description = if path.fragment.is_some() {
                        if path.target.is_empty() {
                            "当前笔记".to_string()
                        } else {
                            path.target.clone()
                        }
                    } else if path.alias.is_some() {
                        path.label.clone()
                    } else {
                        path.label
                            .rsplit_once('/')
                            .map_or("/", |(parent, _)| parent)
                            .to_string()
                    };
                    let needle = typed
                        .split_once('#')
                        .map_or(typed, |(_, fragment)| fragment.trim_start_matches('^'));
                    let ranges: Vec<_> = if needle.is_empty() {
                        vec![]
                    } else {
                        regex::RegexBuilder::new(&regex::escape(needle))
                            .case_insensitive(true)
                            .build()
                            .ok()
                            .map(|pattern| {
                                pattern
                                    .find_iter(&label)
                                    .map(|m| [m.start(), m.end()])
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    lsp_types::CompletionItem {
                        label,
                        label_details: Some(lsp_types::CompletionItemLabelDetails {
                            detail: None,
                            description: Some(description),
                        }),
                        data: Some(serde_json::json!({ "inkstone_match_ranges": ranges })),
                        filter_text: Some(format!(
                            "{} {} {}",
                            path.label,
                            path.target,
                            path.alias.as_deref().unwrap_or("")
                        )),
                        text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                            range: lsp_types::Range::new(
                                text.offset_to_position(if path.markdown {
                                    start
                                } else {
                                    start + 2
                                }),
                                text.offset_to_position(end),
                            ),
                            new_text: if path.markdown {
                                let name = path
                                    .alias
                                    .as_deref()
                                    .unwrap_or_else(|| {
                                        if path.fragment.is_some() {
                                            &path.label
                                        } else {
                                            path.label.rsplit('/').next().unwrap_or(&path.label)
                                        }
                                    })
                                    .replace('\\', "\\\\")
                                    .replace('[', "\\[")
                                    .replace(']', "\\]");
                                const ESCAPE: &percent_encoding::AsciiSet =
                                    &percent_encoding::CONTROLS
                                        .add(b' ')
                                        .add(b'#')
                                        .add(b'%')
                                        .add(b'(')
                                        .add(b')')
                                        .add(b'[')
                                        .add(b']');
                                let mut destination =
                                    percent_encoding::utf8_percent_encode(&path.target, ESCAPE)
                                        .to_string();
                                if let Some(fragment) = &path.fragment {
                                    destination.push('#');
                                    destination.push_str(
                                        &percent_encoding::utf8_percent_encode(fragment, ESCAPE)
                                            .to_string(),
                                    );
                                }
                                format!("[{name}]({destination})")
                            } else {
                                let target = if let Some(fragment) = &path.fragment {
                                    format!("{}#{fragment}", path.target)
                                } else {
                                    path.target.clone()
                                };
                                if let Some(alias) = &path.alias {
                                    format!("{target}|{alias}]]")
                                } else {
                                    format!("{target}]]")
                                }
                            },
                        })),
                        ..Default::default()
                    }
                })
                .collect();
            Ok(lsp_types::CompletionResponse::Array(items))
        });
        cx.spawn(async move |cx| {
            let response = task.await;
            if let Some(editor) = editor {
                let valid = editor
                    .read_with(cx, |state, _| {
                        state.cursor() == offset && state.text() == &snapshot
                    })
                    .unwrap_or(false);
                if !valid {
                    return Ok(lsp_types::CompletionResponse::Array(vec![]));
                }
            }
            response
        })
    }
}

pub fn completion_anchors(source: &str) -> Arc<Vec<(String, String)>> {
    let parsed = inkstone_core::index::parse(source);
    anchors_from_parsed(&parsed)
}
pub fn anchors_from_parsed(parsed: &ParsedNote) -> Arc<Vec<(String, String)>> {
    Arc::new(
        parsed
            .headings
            .iter()
            .map(|h| (h.title.clone(), h.title.clone()))
            .chain(
                parsed
                    .blocks
                    .iter()
                    .map(|b| (format!("^{}", b.id), format!("^{}", b.id))),
            )
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_uri_roundtrip_chinese_and_spaces() {
        let target = "目录/中文 空格#标题";
        assert_eq!(
            decode_uri(&target_uri(target).to_string()).as_deref(),
            Some(target)
        );
        assert_eq!(decode_uri("inkstone:xyz"), None);
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    async fn wiki_completion_ignores_literal_code_and_escaped_openers(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| crate::editor::EditorPane::new("", w, cx));
        for (marked, expected) in [
            ("```md\n[[note@\n```", false),
            ("~~~\n[[note@", false),
            ("    [[note@", false),
            ("> ```\n> [[note@\n> ```", false),
            ("`[[note@`", false),
            ("``a ` [[note@``", false),
            ("<!-- [[note@ -->", false),
            ("\\[[note@", false),
            ("\\\\[[note@", true),
            ("`code` 中文 [[note@", true),
            ("> normal [[note@", true),
        ] {
            let offset = marked.find('@').unwrap();
            let source = marked.replace('@', "");
            let provider =
                WikiCompletions(Rc::new(RefCell::new(Arc::new(vec!["note".into()]))), None);
            let task = handle
                .update(cx, |_, w, cx| {
                    provider.completions(
                        &Rope::from(source.clone()),
                        offset,
                        lsp_types::CompletionContext {
                            trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                            trigger_character: None,
                        },
                        w,
                        cx,
                    )
                })
                .unwrap();
            let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
                panic!("array expected");
            };
            assert_eq!(!items.is_empty(), expected, "{marked}");
        }
    }
    #[gpui::test]
    async fn heading_and_block_completions_keep_fragments_and_local_source(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| crate::editor::EditorPane::new("", w, cx));
        for (source, markdown, expected) in [
            ("[[note#章]]", false, "[[note#章节 1]]"),
            ("![[note#^bl]]", false, "![[note#^block]]"),
            ("[[note#章]]", true, "note.md#章节 1"),
            ("# 当前/标题\n\n[[#当前]]", true, "#当前/标题"),
            (
                "# 当前/标题\n\n[[#当前]]",
                false,
                "# 当前/标题\n\n[[#当前/标题]]",
            ),
        ] {
            let provider = WikiCompletions(
                Rc::new(RefCell::new(Arc::new(vec![CompletionPath {
                    label: "folder/note".into(),
                    target: if markdown { "note.md" } else { "note" }.into(),
                    markdown,
                    anchors: completion_anchors("# 章节 1\n\nText ^block\n"),
                    ..Default::default()
                }]))),
                None,
            );
            let task = handle
                .update(cx, |p, w, cx| {
                    p.editor.update(cx, |s, cx| s.set_value(source, w, cx));
                    provider.completions(
                        &Rope::from(source),
                        source.len() - 2,
                        lsp_types::CompletionContext {
                            trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                            trigger_character: None,
                        },
                        w,
                        cx,
                    )
                })
                .unwrap();
            let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
                panic!("array expected");
            };
            assert_eq!(items.len(), 1, "{source}");
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = items[0].text_edit.clone() else {
                panic!("edit expected");
            };
            handle
                .update(cx, |p, w, cx| {
                    p.editor
                        .update(cx, |s, cx| s.apply_lsp_edits(&vec![edit], w, cx));
                    let value = p.editor.read(cx).value();
                    if markdown {
                        let parsed = inkstone_core::index::parse(&value);
                        assert!(value.contains(if expected.starts_with('#') {
                            "](#"
                        } else {
                            ".md#"
                        }));
                        assert_eq!(
                            percent_encoding::percent_decode_str(&parsed.standard_links[0].0)
                                .decode_utf8()
                                .unwrap(),
                            expected
                        );
                    } else {
                        assert_eq!(value.as_ref(), expected);
                    }
                })
                .unwrap();
        }
    }
    #[gpui::test]
    async fn alias_completion_preserves_label_and_real_destination(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| crate::editor::EditorPane::new("", w, cx));
        for markdown in [false, true] {
            let source = "[[别名]]";
            let target = if markdown {
                "../目录/真实.md"
            } else {
                "真实"
            };
            let provider = WikiCompletions(
                Rc::new(RefCell::new(Arc::new(vec![CompletionPath {
                    label: "目录/真实".into(),
                    target: target.into(),
                    markdown,
                    alias: Some("项目别名".into()),
                    ..Default::default()
                }]))),
                None,
            );
            let task = handle
                .update(cx, |p, w, cx| {
                    p.editor.update(cx, |s, cx| s.set_value(source, w, cx));
                    provider.completions(
                        &Rope::from(source),
                        source.len() - 2,
                        lsp_types::CompletionContext {
                            trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                            trigger_character: None,
                        },
                        w,
                        cx,
                    )
                })
                .unwrap();
            let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
                panic!("array expected");
            };
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].label, "项目别名");
            assert_eq!(
                items[0]
                    .label_details
                    .as_ref()
                    .unwrap()
                    .description
                    .as_deref(),
                Some("目录/真实")
            );
            assert_eq!(
                items[0].data.as_ref().unwrap()["inkstone_match_ranges"],
                serde_json::json!([[6, 12]])
            );
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = items[0].text_edit.clone() else {
                panic!("edit expected");
            };
            handle
                .update(cx, |p, w, cx| {
                    p.editor
                        .update(cx, |s, cx| s.apply_lsp_edits(&vec![edit], w, cx));
                    let value = p.editor.read(cx).value();
                    if markdown {
                        assert!(value.starts_with("[项目别名]("));
                        let parsed = inkstone_core::index::parse(&value);
                        assert_eq!(
                            percent_encoding::percent_decode_str(&parsed.standard_links[0].0)
                                .decode_utf8()
                                .unwrap(),
                            target
                        );
                    } else {
                        assert_eq!(value.as_ref(), "[[真实|项目别名]]");
                    }
                })
                .unwrap();
        }
    }

    #[gpui::test]
    async fn completion_searches_full_path_but_inserts_configured_target(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("", window, cx));
        for (typed, target, markdown) in [
            ("项目/", "计划", false),
            ("../", "../项目/计划", false),
            ("计划", "项目/计划", false),
            ("项目/", "../项目/计 划#1.md", true),
            ("项目/", "../项目/image #1.png", true),
        ] {
            let embed = if target.ends_with(".png") { "!" } else { "" };
            let source = format!("中文😀 {embed}[[{typed}]]");
            let offset = source.len() - 2;
            let provider = WikiCompletions(
                Rc::new(RefCell::new(Arc::new(vec![CompletionPath {
                    label: "项目/计划".into(),
                    target: target.into(),
                    markdown,
                    alias: None,
                    ..Default::default()
                }]))),
                None,
            );
            let task = handle
                .update(cx, |pane, window, cx| {
                    pane.editor
                        .update(cx, |s, cx| s.set_value(source.clone(), window, cx));
                    provider.completions(
                        &Rope::from(source.clone()),
                        offset,
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
                panic!("expected array");
            };
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].label, "计划");
            assert_eq!(
                items[0]
                    .label_details
                    .as_ref()
                    .unwrap()
                    .description
                    .as_deref(),
                Some("项目")
            );
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = items[0].text_edit.clone() else {
                panic!("expected edit");
            };
            handle
                .update(cx, |pane, window, cx| {
                    pane.editor
                        .update(cx, |s, cx| s.apply_lsp_edits(&vec![edit], window, cx));
                    let result = pane.editor.read(cx).value();
                    if markdown {
                        assert!(result.starts_with(&format!("中文😀 {embed}[计划](")));
                        let parsed = inkstone_core::index::parse(&result);
                        assert_eq!(parsed.destinations.len(), 1);
                        assert_eq!(
                            percent_encoding::percent_decode_str(&parsed.destinations[0].1)
                                .decode_utf8()
                                .unwrap(),
                            target
                        );
                        assert!(!result.contains("]]"));
                    } else {
                        assert_eq!(result.as_ref(), format!("中文😀 [[{target}]]"));
                    }
                })
                .unwrap();
        }
    }

    #[gpui::test]
    async fn completion_reuses_zero_one_or_two_auto_closers(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("", window, cx));
        for suffix in ["", "]", "]]"] {
            let prefix = "中文😀 [[";
            let source = format!("{prefix}{suffix}");
            let provider =
                WikiCompletions(Rc::new(RefCell::new(Arc::new(vec!["/欢迎".into()]))), None);
            let task = handle
                .update(cx, |pane, window, cx| {
                    pane.editor
                        .update(cx, |state, cx| state.set_value(source.clone(), window, cx));
                    provider.completions(
                        &Rope::from(source.clone()),
                        prefix.len(),
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
                panic!("expected array")
            };
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = items[0].text_edit.clone() else {
                panic!("expected text edit")
            };
            handle
                .update(cx, |pane, window, cx| {
                    pane.editor.update(cx, |state, cx| {
                        state.apply_lsp_edits(&vec![edit], window, cx)
                    });
                    assert_eq!(pane.editor.read(cx).value().as_ref(), "中文😀 [[/欢迎]]");
                })
                .unwrap();
        }
    }

    #[gpui::test]
    async fn stale_completion_result_is_discarded(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("[[笔", window, cx));
        let task = handle
            .update(cx, |pane, window, cx| {
                let editor = pane.editor.clone();
                editor.update(cx, |state, cx| state.set_selected_range(5..5, cx));
                let provider = WikiCompletions(
                    Rc::new(RefCell::new(Arc::new(vec!["/笔记".into()]))),
                    Some(editor.downgrade()),
                );
                let task = provider.completions(
                    &Rope::from("[[笔"),
                    5,
                    lsp_types::CompletionContext {
                        trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                        trigger_character: None,
                    },
                    window,
                    cx,
                );
                editor.update(cx, |state, cx| state.set_value("另一个文档", window, cx));
                task
            })
            .unwrap();
        let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
            panic!("array expected")
        };
        assert!(items.is_empty());
    }

    #[gpui::test]
    async fn completions_and_definitions_use_component_unicode_coordinates(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("", window, cx));
        let text = Rope::from("中😀 [[笔");
        let provider = WikiCompletions(
            Rc::new(RefCell::new(Arc::new(vec![
                "/目录/笔记".into(),
                "/其他".into(),
            ]))),
            None,
        );
        let task = handle
            .update(cx, |_, window, cx| {
                provider.completions(
                    &text,
                    text.len(),
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
            panic!("array expected")
        };
        assert_eq!(items.len(), 1);
        let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
            panic!("edit expected")
        };
        assert_eq!(text.position_to_offset(&edit.range.start), "中😀 [[".len());
        assert_eq!(edit.new_text, "/目录/笔记]]");
        let source = "中😀 [[目录/笔记|标签]]";
        let provider = WikiDefinitions(Rc::new(RefCell::new((
            source.into(),
            inkstone_core::index::parse(source),
        ))));
        let text = Rope::from(source);
        let task = handle
            .update(cx, |_, window, cx| {
                provider.definitions(&text, "中😀 [[".len(), window, cx)
            })
            .unwrap();
        let links = task.await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            decode_uri(&links[0].target_uri.to_string()).unwrap(),
            "目录/笔记"
        );
        let source = "中文😀[^a]\n\n[^a]: definition";
        let provider = WikiDefinitions(Rc::new(RefCell::new((
            source.into(),
            inkstone_core::index::parse(source),
        ))));
        let text = Rope::from(source);
        let offset = source.find("[^a]").unwrap() + 2;
        let task = handle
            .update(cx, |_, window, cx| {
                provider.definitions(&text, offset, window, cx)
            })
            .unwrap();
        let links = task.await.unwrap();
        assert_eq!(
            links[0].target_uri.as_str(),
            format!("inkstone-footnote:{offset}")
        );
        assert_eq!(
            text.position_to_offset(&links[0].origin_selection_range.unwrap().start),
            offset - 2
        );
    }
}
