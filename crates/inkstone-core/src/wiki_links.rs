//! Shared source ranges for Wiki references in prose, including formatted aliases.
use markdown_parser::mdast::Node;
use std::ops::Range;

pub(crate) struct Link {
    pub source: Range<usize>,
    pub target: Range<usize>,
    pub label: Range<usize>,
}

pub(crate) fn scan(source: &str, ast: &Node) -> Vec<Link> {
    fn append(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = ranges.last_mut()
            && last.end == range.start
        {
            last.end = range.end;
        } else {
            ranges.push(range);
        }
    }
    fn prose(node: &Node, ranges: &mut Vec<Range<usize>>) {
        let Some(p) = node.position() else { return };
        if matches!(node, Node::Text(_)) {
            append(ranges, p.start.offset..p.end.offset);
            return;
        }
        let Some(children) = node.children() else {
            return;
        };
        // Include formatting delimiters, but leave code, math, link destinations,
        // and block boundaries outside the contiguous prose ranges.
        let formatted = matches!(node, Node::Emphasis(_) | Node::Strong(_) | Node::Delete(_));
        if formatted && let Some(first) = children.first().and_then(Node::position) {
            append(ranges, p.start.offset..first.start.offset);
        }
        for child in children {
            prose(child, ranges);
        }
        if formatted && let Some(last) = children.last().and_then(Node::position) {
            append(ranges, last.end.offset..p.end.offset);
        }
    }
    if !source.contains("[[") {
        return vec![];
    }
    let mut ranges = vec![];
    prose(ast, &mut ranges);
    let mut links = vec![];
    for range in ranges {
        let raw = &source[range.clone()];
        let mut offset = 0;
        while let Some(start) = raw[offset..].find("[[") {
            let start = offset + start;
            if source[..range.start + start]
                .bytes()
                .rev()
                .take_while(|b| *b == b'\\')
                .count()
                % 2
                == 1
            {
                offset = start + 2;
                continue;
            }
            let content = start + 2;
            let Some(end) = raw[content..].find("]]").map(|n| content + n) else {
                break;
            };
            if let Some(newline) = raw[content..end].rfind(['\r', '\n']) {
                offset = content + newline + 1;
                continue;
            }
            let separator = raw[content..end].find('|').map(|i| content + i);
            let target_end = separator.unwrap_or(end);
            if !raw[content..target_end]
                .trim_matches(|c: char| c.is_whitespace() || c == '\u{1}')
                .is_empty()
            {
                links.push(Link {
                    source: range.start + start..range.start + end + 2,
                    target: range.start + content..range.start + target_end,
                    label: range.start + separator.map_or(content, |i| i + 1)..range.start + end,
                });
            }
            offset = end + 2;
        }
    }
    links
}
