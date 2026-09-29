//! Source-based live styles derived from the Markdown syntax tree.
use std::ops::Range;

/// Heading scales from the installed Obsidian 1.13.7 default stylesheet.
pub const HEADING_SCALES: [f32; 6] = [1.618, 1.462, 1.318, 1.188, 1.076, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
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
}
impl Span {
    pub fn active(&self, selection: &Range<usize>) -> bool {
        if selection.is_empty() {
            self.source.start <= selection.start && selection.start <= self.source.end
        } else {
            selection.start < self.source.end && selection.end > self.source.start
        }
    }
}

/// Keep unaffected styles at their source positions while a newer parse is pending.
pub fn rebase_spans(before: &str, after: &str, spans: Vec<Span>) -> Vec<Span> {
    if before == after {
        return spans;
    }
    let start = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let suffix = before[start..]
        .chars()
        .rev()
        .zip(after[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let end = before.len() - suffix;
    let delta = after.len() as isize - before.len() as isize;
    let shifted =
        |r: &Range<usize>| r.start.saturating_add_signed(delta)..r.end.saturating_add_signed(delta);
    spans
        .into_iter()
        .filter_map(|mut span| {
            if span.source.start <= end && start <= span.source.end {
                return None;
            }
            if span.source.start >= end {
                span.source = shifted(&span.source);
                span.content = shifted(&span.content);
                span.markers = span.markers.iter().map(shifted).collect();
            }
            Some(span)
        })
        .collect()
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
    use markdown_parser::mdast::Node;
    fn boundary(node: &Node) -> Option<Range<usize>> {
        node.position().map(|p| p.start.offset..p.end.offset)
    }
    fn walk(node: &Node, text: &str, out: &mut Vec<Span>) {
        let Some(range) = boundary(node) else {
            return;
        };
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
                });
            }
            return;
        }
        if matches!(node, Node::Code(_) | Node::Yaml(_) | Node::Html(_)) {
            return;
        }
        if let Node::Text(_) = node {
            let raw = &text[range.clone()];
            let mut i = 0;
            while i < raw.len() {
                let rest = &raw[i..];
                if rest.starts_with('\\') {
                    i += 1;
                    if i < raw.len() {
                        i += raw[i..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                let pair = if rest.starts_with("[[") {
                    Some(("[[", "]]", Kind::WikiLink))
                } else {
                    None
                };
                if let Some((open, close, kind)) = pair {
                    if text[..range.start + i]
                        .bytes()
                        .rev()
                        .take_while(|b| *b == b'\\')
                        .count()
                        % 2
                        == 1
                    {
                        i += open.len();
                        continue;
                    }
                    let start = i + open.len();
                    if let Some(end) = raw[start..]
                        .find(close)
                        .map(|p| p + start)
                        .filter(|end| *end > start && !raw[start..*end].contains(['\r', '\n']))
                    {
                        let label = if kind == Kind::WikiLink {
                            raw[start..end].find('|').map_or(start, |p| start + p + 1)
                        } else {
                            start
                        };
                        out.push(Span {
                            kind,
                            source: range.start + i..range.start + end + close.len(),
                            content: range.start + label..range.start + end,
                            markers: vec![
                                range.start + i..range.start + label,
                                range.start + end..range.start + end + close.len(),
                            ],
                        });
                        i = end + close.len();
                        continue;
                    }
                }
                i += rest.chars().next().unwrap().len_utf8();
            }
        }
        if let Some(children) = children {
            for child in children {
                walk(child, text, out);
            }
        }
    }
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let mut result = vec![];
    if let Ok(node) = markdown_parser::to_mdast(text, &options) {
        walk(&node, text, &mut result);
        fn excluded(node: &Node, out: &mut Vec<Range<usize>>) {
            if matches!(
                node,
                Node::Code(_) | Node::InlineCode(_) | Node::Html(_) | Node::Yaml(_)
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
        excluded(&node, &mut blocked);
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
    fn styles_follow_distant_unicode_edits_while_parsing_is_pending() {
        let before = "**one**\ntext\n[[far|alias]]";
        let after = "**one**\n文本😀\n[[far|alias]]";
        let retained = rebase_spans(before, after, spans(before));
        assert_eq!(retained.len(), 2);
        assert_eq!(&after[retained[0].source.clone()], "**one**");
        assert_eq!(&after[retained[1].content.clone()], "alias");
        let changed = rebase_spans(before, "**oXne**\ntext\n[[far|alias]]", spans(before));
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].kind, Kind::WikiLink);
    }
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
    #[test]
    fn code_fences_and_escaped_markers_stay_source() {
        let literal = spans("```md\n**不是粗体**\n```\n\\[[不是链接]]\n\n    **代码**");
        assert!(literal.is_empty(), "{literal:?}");
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
    fn task_line_formatting_keeps_quote_indentation_and_crlf() {
        let source = "> - [ ] 第一项😀\r\n\t2. 第二项\r\n最后一行";
        let (range, changed) = toggle_task_lines(source, 0..source.find("最后").unwrap()).unwrap();
        assert_eq!(changed, "> - [x] 第一项😀\r\n\t2. [ ] 第二项\r\n");
        assert_eq!(&source[range], "> - [ ] 第一项😀\r\n\t2. 第二项\r\n");
        assert_eq!(toggle_task_lines("", 0..0).unwrap().1, "- [ ] ");
        assert!(toggle_task_lines("中", 1..1).is_none());
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
