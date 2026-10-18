//! Source-based live styles derived from the Markdown syntax tree.
use std::ops::Range;

/// Heading scales from the installed Obsidian 1.13.7 default stylesheet.
pub const HEADING_SCALES: [f32; 6] = [1.618, 1.462, 1.318, 1.188, 1.076, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Comment,
    Rule,
    QuoteMarker,
    ListMarker,
    QuoteContinuation,
    Heading,
    Strong,
    Emphasis,
    Strike,
    Link,
    Highlight,
    Code,
    WikiLink,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub kind: Kind,
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub markers: Vec<Range<usize>>,
    /// Enclosing lists. Reading mode indents one rem per list.
    pub list_depth: usize,
}
impl Span {
    /// Stable source prefixes for every logical line of a heading's content.
    pub fn heading_line_anchors(&self, text: &str) -> Vec<Range<usize>> {
        if self.kind != Kind::Heading {
            return vec![];
        }
        let Some(raw) = text.get(self.source.start..self.content.end) else {
            return vec![];
        };
        let mut start = self.source.start;
        raw.split_inclusive('\n')
            .filter_map(|line| {
                let offset = start;
                start += line.len();
                let line = line.trim_end_matches(['\r', '\n']);
                let prefix = line
                    .bytes()
                    .take_while(|b| matches!(b, b'#' | b' ' | b'\t'))
                    .count();
                let length = if prefix > 0 {
                    prefix
                } else {
                    line.chars().next()?.len_utf8()
                };
                Some(offset..offset + length)
            })
            .collect()
    }

    /// Start of a Setext underline's source line, including container prefixes.
    pub fn setext_line(&self, text: &str) -> Option<usize> {
        if self.kind != Kind::Heading {
            return None;
        }
        let suffix = text.get(self.content.end..self.source.end)?;
        let start = self.content.end + suffix.rfind('\n')? + 1;
        let mut line = text.get(start..self.source.end)?.trim();
        while let Some(rest) = line.strip_prefix('>') {
            line = rest.trim_start();
        }
        let marker = line.as_bytes().first().copied()?;
        (matches!(marker, b'=' | b'-') && line.bytes().all(|b| b == marker)).then_some(start)
    }

    pub fn active(&self, selection: &Range<usize>) -> bool {
        if selection.is_empty() {
            self.source.start <= selection.start && selection.start <= self.source.end
        } else {
            selection.start < self.source.end && selection.end > self.source.start
        }
    }
}

/// One undoable source edit over the selected lines, preserving indentation and line endings.
pub fn toggle_task_lines(text: &str, selection: Range<usize>) -> Option<(Range<usize>, String)> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let end_cursor = if selection.end > selection.start && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let end = text[end_cursor..]
        .find('\n')
        .map_or(text.len(), |i| end_cursor + i + 1);
    static PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^(?P<indent>(?:[ \t]*>[ \t]*)*[ \t]*)(?P<bullet>(?:[-+*]|[0-9]{1,9}[.)])[ \t]+)?",
        )
        .unwrap()
    });
    let mut result = String::new();
    let raw = &text[start..end];
    let lines: Vec<_> = if raw.is_empty() {
        vec![""]
    } else {
        raw.split_inclusive('\n').collect()
    };
    for line in lines {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let parts = PREFIX.captures(body)?;
        let prefix = parts.get(0)?.as_str();
        let rest = &body[prefix.len()..];
        result.push_str(prefix);
        if parts.name("bullet").is_none() {
            result.push_str("- ");
        }
        if let Some((marker, checked)) = crate::index::task_box_marker(rest) {
            result.push_str(if !checked { "[x]" } else { "[ ]" });
            result.push_str(&rest[marker.end + 1..]);
        } else {
            result.push_str("[ ] ");
            result.push_str(rest);
        }
        result.push_str(ending);
    }
    Some((start..end, result))
}

pub fn spans(text: &str) -> Vec<Span> {
    spans_snapshot(&crate::syntax::Snapshot::new(text))
}

pub fn spans_snapshot(snapshot: &crate::syntax::Snapshot) -> Vec<Span> {
    let text = snapshot.structural.as_ref();
    use markdown_parser::mdast::Node;
    fn boundary(node: &Node) -> Option<Range<usize>> {
        node.position().map(|p| p.start.offset..p.end.offset)
    }
    fn walk(node: &Node, text: &str, out: &mut Vec<Span>, quote_depth: usize, list_depth: usize) {
        let Some(range) = boundary(node) else {
            return;
        };
        if matches!(node, Node::ThematicBreak(_)) {
            let start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
            let end = text[range.start..]
                .find(['\r', '\n'])
                .map_or(text.len(), |i| range.start + i);
            out.push(Span {
                kind: Kind::Rule,
                source: start..end,
                content: range.clone(),
                markers: std::iter::once(range).collect(),
                list_depth: 0,
            });
            return;
        }
        // Use syntax nodes so code, escaped markers and thematic breaks remain literal.
        let item = text[range.clone()].trim_start_matches([' ', '\t']);
        if matches!(node, Node::ListItem(_)) && item.starts_with(['*', '-', '+']) {
            // Tab-indented items may start before their marker in the syntax tree.
            let start = range.end - item.len();
            let marker = start..start + 1;
            out.push(Span {
                kind: Kind::ListMarker,
                source: marker.clone(),
                content: marker,
                markers: vec![],
                list_depth: 0,
            });
        }
        let list_depth = list_depth + usize::from(matches!(node, Node::List(_)));
        let quote_depth = quote_depth + usize::from(matches!(node, Node::Blockquote(_)));
        if matches!(node, Node::Blockquote(_)) {
            let mut offset = range.start;
            for part in text[range.clone()].split_inclusive('\n') {
                let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
                let end = offset + part.trim_end_matches(['\r', '\n']).len();
                let line = &text[start..end];
                let mut rest = line;
                let mut depth = 0;
                let mut found = false;
                loop {
                    rest = rest.trim_start_matches([' ', '\t']);
                    if let Some(next) = rest.strip_prefix('>') {
                        depth += 1;
                        if depth == quote_depth {
                            found = true;
                            let marker = start + line.len() - rest.len();
                            out.push(Span {
                                kind: Kind::QuoteMarker,
                                source: start..end,
                                content: marker..marker + 1,
                                markers: std::iter::once(marker..marker + 1).collect(),
                                list_depth,
                            });
                            break;
                        }
                        rest = next;
                    } else {
                        let prefix = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
                        let marker = if prefix > 0
                            && rest
                                .as_bytes()
                                .get(prefix)
                                .is_some_and(|b| matches!(b, b'.' | b')'))
                        {
                            prefix + 1
                        } else if rest.starts_with(['-', '+', '*']) {
                            1
                        } else {
                            break;
                        };
                        if !rest
                            .as_bytes()
                            .get(marker)
                            .is_some_and(|b| matches!(b, b' ' | b'\t'))
                        {
                            break;
                        }
                        rest = &rest[marker..];
                    }
                }
                if !found && quote_depth == 1 && end > start {
                    out.push(Span {
                        kind: Kind::QuoteContinuation,
                        source: start..end,
                        content: start..start,
                        markers: vec![],
                        list_depth,
                    });
                }
                offset += part.len();
            }
        }
        let children = node.children();
        let content = children
            .and_then(|children| {
                Some(boundary(children.first()?)?.start..boundary(children.last()?)?.end)
            })
            .or_else(|| matches!(node, Node::Heading(_)).then_some(range.end..range.end));
        let kind = match node {
            Node::Heading(_) => Some(Kind::Heading),
            Node::Strong(_) => Some(Kind::Strong),
            Node::Emphasis(_) => Some(Kind::Emphasis),
            Node::Delete(_) => Some(Kind::Strike),
            Node::Link(_) | Node::LinkReference(_) => Some(Kind::Link),
            _ => None,
        };
        if let (Some(kind), Some(content)) = (kind, content) {
            let mut markers = vec![];
            if range.start < content.start {
                markers.push(range.start..content.start);
            }
            if content.end < range.end {
                markers.push(content.end..range.end);
            }
            out.push(Span {
                kind,
                source: range.clone(),
                content,
                markers,
                list_depth: 0,
            });
        }
        if let Node::InlineCode(_) = node {
            let raw = &text[range.clone()];
            let count = raw.bytes().take_while(|b| *b == b'`').count();
            if count > 0 && raw.len() > 2 * count {
                out.push(Span {
                    kind: Kind::Code,
                    source: range.clone(),
                    content: range.start + count..range.end - count,
                    markers: vec![
                        range.start..range.start + count,
                        range.end - count..range.end,
                    ],
                    list_depth: 0,
                });
            }
            return;
        }
        if matches!(
            node,
            Node::Code(_) | Node::Yaml(_) | Node::Html(_) | Node::Math(_) | Node::InlineMath(_)
        ) {
            return;
        }
        if let Some(children) = children {
            for child in children {
                walk(child, text, out, quote_depth, list_depth);
            }
        }
    }
    let mut result: Vec<_> = snapshot
        .comments
        .iter()
        .map(|comment| Span {
            kind: Kind::Comment,
            source: comment.range.clone(),
            content: comment.range.clone(),
            markers: vec![],
            list_depth: 0,
        })
        .collect();
    if let Some(node) = snapshot.ast.as_deref() {
        walk(node, text, &mut result, 0, 0);
        result.extend(
            crate::wiki_links::scan(text, node)
                .into_iter()
                .map(|link| Span {
                    kind: Kind::WikiLink,
                    markers: vec![
                        link.source.start..link.label.start,
                        link.label.end..link.source.end,
                    ],
                    source: link.source,
                    content: link.label,
                    list_depth: 0,
                }),
        );
        if !text.contains("==") {
            return result;
        }
        fn excluded(node: &Node, out: &mut Vec<Range<usize>>) {
            if matches!(
                node,
                Node::Code(_)
                    | Node::InlineCode(_)
                    | Node::Html(_)
                    | Node::Yaml(_)
                    | Node::Math(_)
                    | Node::InlineMath(_)
            ) {
                if let Some(r) = boundary(node) {
                    out.push(r);
                }
                return;
            }
            if let Some(children) = node.children() {
                for child in children {
                    excluded(child, out);
                }
            }
        }
        let mut blocked = vec![];
        excluded(node, &mut blocked);
        let mut open = None;
        for (offset, _) in text.match_indices("==") {
            if text[..offset]
                .bytes()
                .rev()
                .take_while(|b| *b == b'\\')
                .count()
                % 2
                == 1
                || blocked
                    .get(blocked.partition_point(|r| r.end <= offset))
                    .is_some_and(|r| r.contains(&offset))
            {
                continue;
            }
            let can_open = text[offset + 2..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace());
            let can_close = text[..offset]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace());
            if let Some(start) = open.take() {
                if text[start + 2..offset].contains(['\r', '\n']) {
                    open = can_open.then_some(offset);
                    continue;
                }
                if !can_close {
                    open = Some(start);
                    continue;
                }
                if offset > start + 2 {
                    result.push(Span {
                        kind: Kind::Highlight,
                        source: start..offset + 2,
                        content: start + 2..offset,
                        markers: vec![start..start + 2, offset..offset + 2],
                        list_depth: 0,
                    });
                }
            } else if can_open {
                open = Some(offset);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_ranges_are_source_bytes_and_source_is_unchanged() {
        let s = "# 中文\r\n**粗体😀** 和 `代码` [[笔记]]\n";
        let items = spans(s);
        assert_eq!(items.len(), 4);
        assert_eq!(&s[items[1].content.clone()], "粗体😀");
        assert_eq!(&s[items[3].content.clone()], "笔记");
        for item in items {
            assert!(item.active(&(item.content.start..item.content.start)));
        }
    }

    fn styled(source: &str, kind: Kind) -> Vec<&str> {
        spans(source)
            .into_iter()
            .filter(|span| span.kind == kind)
            .map(|span| &source[span.content.clone()])
            .collect()
    }

    #[test]
    fn cjk_punctuation_does_not_block_adjacent_emphasis() {
        for (source, kind, content) in [
            ("**叶变换对，**", Kind::Strong, "叶变换对，"),
            ("**叶变换对，**这种情况下", Kind::Strong, "叶变换对，"),
            ("前文**叶变换对，**这种", Kind::Strong, "叶变换对，"),
            ("**叶变换对。**这是下一句", Kind::Strong, "叶变换对。"),
            ("这是**“叶变换对”**的说明", Kind::Strong, "“叶变换对”"),
            ("*叶变换对，*这种", Kind::Emphasis, "叶变换对，"),
            ("**「重要」**的", Kind::Strong, "「重要」"),
            ("~~删除，~~后续", Kind::Strike, "删除，"),
            ("**hello,**这", Kind::Strong, "hello,"),
            ("***叶变换对，***这种", Kind::Strong, "叶变换对，"),
        ] {
            assert_eq!(styled(source, kind), vec![content], "{source:?}");
        }
        let nested = "**叶变换对，**这种情况下";
        let strong = spans(nested)
            .into_iter()
            .find(|span| span.kind == Kind::Strong)
            .unwrap();
        assert_eq!(
            strong
                .markers
                .iter()
                .map(|marker| &nested[marker.clone()])
                .collect::<Vec<_>>(),
            ["**", "**"]
        );
        assert_eq!(styled("这是**粗体**文字", Kind::Strong), vec!["粗体"]);
        assert_eq!(styled("**hello,** world", Kind::Strong), vec!["hello,"]);
        assert!(styled("**hello,**world", Kind::Strong).is_empty());
        assert!(styled("`**叶变换对，**这种`", Kind::Strong).is_empty());
        assert!(styled("__叶变换对，__这种", Kind::Strong).is_empty());
        // `***...***` is emphasis around strong, so the emphasis range keeps the inner markers.
        assert_eq!(
            styled("***叶变换对，***这种", Kind::Emphasis),
            vec!["**叶变换对，**"]
        );
    }
    #[test]
    fn list_markers_follow_syntax_and_preserve_nested_source_offsets() {
        let source = "* 中文\n  - nested\n\n+ other\n\n> * quoted\n\n1. ordered\n\n***\n\n\\* escaped\n\n```md\n* code\n```\n\n    * indented item";
        let markers: Vec<_> = spans(source)
            .into_iter()
            .filter(|span| span.kind == Kind::ListMarker)
            .map(|span| span.content.start)
            .collect();
        assert_eq!(
            markers,
            [
                0,
                source.find("- nested").unwrap(),
                source.find("+ other").unwrap(),
                source.find("* quoted").unwrap(),
                source.find("* indented item").unwrap(),
            ]
        );
    }

    #[test]
    fn tab_indented_multilevel_bullets_keep_marker_offsets() {
        for indent in ["\t", "  ", "    "] {
            let source = format!(
                "* 身份认证\n{indent}* 用户\n{indent}{indent}* 租户\n* 生命周期\n{indent}* 实例\n{indent}* Region"
            );
            let markers: Vec<_> = spans(&source)
                .into_iter()
                .filter(|span| span.kind == Kind::ListMarker)
                .map(|span| span.content)
                .collect();
            let expected: Vec<_> = source.match_indices('*').map(|(i, _)| i..i + 1).collect();
            assert_eq!(markers, expected, "indent: {indent:?}");
        }
    }

    #[test]
    fn code_fences_and_escaped_markers_stay_source() {
        let literal = spans("```md\n**不是粗体**\n```\n\\[[不是链接]]");
        assert!(literal.is_empty(), "{literal:?}");
        // Four spaces after a blank line are indentation, not an unlabelled code block.
        assert!(
            spans("前文\n\n    **粗体**")
                .iter()
                .any(|span| span.kind == Kind::Strong)
        );
        assert_eq!(spans("`**literal**` [[真链接]]").len(), 2);
    }
    #[test]
    fn nested_styles_aliases_and_multitick_code_keep_source_ranges() {
        let source =
            "***粗体斜体*** ~~删除😀~~ ==高亮== [链接](目标.md) [[目标|别名]] ``含 ` 的代码``";
        let all = spans(source);
        for kind in [
            Kind::Strong,
            Kind::Emphasis,
            Kind::Strike,
            Kind::Highlight,
            Kind::Link,
            Kind::WikiLink,
            Kind::Code,
        ] {
            assert!(all.iter().any(|s| s.kind == kind));
        }
        let wiki = all.iter().find(|s| s.kind == Kind::WikiLink).unwrap();
        assert_eq!(&source[wiki.content.clone()], "别名");
        let code = all.iter().find(|s| s.kind == Kind::Code).unwrap();
        assert_eq!(&source[code.content.clone()], "含 ` 的代码");
        // Without a blank line this is paragraph continuation, not an indented code block.
        assert!(
            spans("正文\n    **继续粗体**")
                .iter()
                .any(|s| s.kind == Kind::Strong)
        );
    }
    #[test]
    fn wiki_scanning_skips_prose_and_recovers_after_incomplete_links() {
        for newline in ["\n", "\r\n", "\r"] {
            let source = format!(
                "中文😀 e\u{301} [普通] \\[[转义]] \\\\[[偶数转义]] [[]] [[目标|别名😀]] [[未完成{newline}[[后续]] `[[代码]]` [[未闭合 [[再开"
            );
            let wiki: Vec<_> = spans(&source)
                .into_iter()
                .filter(|span| span.kind == Kind::WikiLink)
                .collect();
            assert_eq!(
                wiki.iter()
                    .map(|span| &source[span.content.clone()])
                    .collect::<Vec<_>>(),
                ["偶数转义", "别名😀", "后续"],
                "{source:?}"
            );
            for span in wiki {
                assert_eq!(&source[span.markers[1].clone()], "]]", "{source:?}");
                assert_eq!(span.source.start, span.markers[0].start);
                assert_eq!(span.content.end, span.markers[1].start);
            }
        }
        let source = format!("{}\n[[最后|中文😀]]", "[[未闭合 ".repeat(512));
        let wiki: Vec<_> = spans(&source)
            .into_iter()
            .filter(|span| span.kind == Kind::WikiLink)
            .collect();
        assert_eq!(wiki.len(), 1);
        assert_eq!(&source[wiki[0].content.clone()], "中文😀");
    }
    #[test]
    fn task_line_formatting_keeps_quote_indentation_and_crlf() {
        let source = "> - [ ] 第一项😀\r\n\t2. 第二项\r\n最后一行";
        let (range, changed) = toggle_task_lines(source, 0..source.find("最后").unwrap()).unwrap();
        assert_eq!(changed, "> - [x] 第一项😀\r\n\t2. [ ] 第二项\r\n");
        assert_eq!(&source[range], "> - [ ] 第一项😀\r\n\t2. 第二项\r\n");
        assert_eq!(toggle_task_lines("", 0..0).unwrap().1, "- [ ] ");
        assert!(toggle_task_lines("中", 1..1).is_none());
    }
    #[test]
    fn setext_underlines_are_whole_lines_and_exclude_rules_and_code() {
        for source in [
            "标题\n===",
            "标题\r\n---",
            "> 标题\n> ---",
            "- 标题\n  ===",
            "第一行\n第二行\n---",
        ] {
            let heading = spans(source)
                .into_iter()
                .find(|span| span.kind == Kind::Heading)
                .unwrap();
            assert_eq!(
                heading.setext_line(source),
                Some(source.rfind('\n').unwrap() + 1)
            );
        }
        for source in ["# 标题", "---\n", "```\n标题\n===\n```", "正文\n\n---"] {
            assert!(
                spans(source)
                    .iter()
                    .all(|span| span.setext_line(source).is_none())
            );
        }
    }
    #[test]
    fn heading_line_anchors_cover_content_but_not_setext_underlines() {
        for source in [
            "中文\n第二行\n===",
            "中文\r\n第二行\r\n---",
            "> 中文\n> 第二行\n> ===",
            "- 中文\n  第二行\n  ---",
        ] {
            let heading = spans(source)
                .into_iter()
                .find(|span| span.kind == Kind::Heading)
                .unwrap();
            let anchors = heading.heading_line_anchors(source);
            assert_eq!(anchors.len(), 2);
            assert_eq!(anchors[0].start, heading.source.start);
            assert_eq!(anchors[1].start, source.find('\n').unwrap() + 1);
            assert!(
                anchors
                    .iter()
                    .all(|anchor| source.is_char_boundary(anchor.end)
                        && !source[anchor.clone()].contains(['\r', '\n']))
            );
        }
        let heading = spans("### 中文")
            .into_iter()
            .find(|span| span.kind == Kind::Heading)
            .unwrap();
        assert_eq!(
            heading.heading_line_anchors("### 中文"),
            std::iter::once(0..4).collect::<Vec<_>>()
        );
    }
    #[test]
    fn quote_markers_follow_ast_depth_and_preserve_quoted_code() {
        let source = "> outer\n> > inner\n> > ```\n> > > literal\n> > ```\n\n- > list\n\n```\n> not quote\n```";
        let quotes: Vec<_> = spans(source)
            .into_iter()
            .filter(|span| span.kind == Kind::QuoteMarker)
            .collect();
        assert_eq!(quotes.len(), 10);
        assert!(
            quotes
                .iter()
                .all(|span| &source[span.content.clone()] == ">")
        );
        assert!(
            !quotes
                .iter()
                .any(|span| span.content.start == source.find("> literal").unwrap())
        );
        assert!(
            !quotes
                .iter()
                .any(|span| span.content.start == source.find("> not quote").unwrap())
        );
        assert!(
            quotes
                .iter()
                .any(|span| span.content.start == source.find("> list").unwrap())
        );
        assert_eq!(
            quotes
                .iter()
                .find(|span| span.content.start == source.find("> list").unwrap())
                .unwrap()
                .list_depth,
            1
        );
        assert!(quotes.iter().any(|span| span.list_depth == 0));
    }
    #[test]
    fn lazy_quote_lines_retain_source_without_invented_markers() {
        let source = "> first\r\n续行\r\n> > nested\r\nnested tail\r\n\r\noutside";
        let continuation: Vec<_> = spans(source)
            .into_iter()
            .filter(|span| span.kind == Kind::QuoteContinuation)
            .collect();
        assert_eq!(continuation.len(), 2);
        assert_eq!(&source[continuation[0].source.clone()], "续行");
        assert_eq!(&source[continuation[1].source.clone()], "nested tail");
        assert!(
            continuation
                .iter()
                .all(|span| span.content.is_empty() && span.markers.is_empty())
        );
        assert!(
            spans("plain\n\n> quote\n\noutside")
                .iter()
                .all(|span| span.kind != Kind::QuoteContinuation)
        );
    }
    #[test]
    fn rules_exclude_setext_yaml_and_fenced_code() {
        let source = "---\ntitle: test\n---\n\nheading\n---\n\n***\n\n> ___\n\n```\n---\n```";
        let rules: Vec<_> = spans(source)
            .into_iter()
            .filter(|span| span.kind == Kind::Rule)
            .collect();
        assert_eq!(rules.len(), 2);
        assert_eq!(&source[rules[0].content.clone()], "***");
        assert_eq!(&source[rules[1].content.clone()], "___");
    }
    #[test]
    fn highlight_handles_nested_formatting_and_escaped_openers() {
        let source = r"\==不高亮== ==**嵌套** [[链接]]== `==代码==`";
        let all = spans(source);
        let marks: Vec<_> = all.iter().filter(|s| s.kind == Kind::Highlight).collect();
        assert_eq!(marks.len(), 1);
        assert_eq!(&source[marks[0].content.clone()], "**嵌套** [[链接]]");
        assert!(all.iter().any(|s| s.kind == Kind::WikiLink));
    }
}
