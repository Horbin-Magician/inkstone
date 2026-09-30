//! Obsidian comments, with byte-preserving input for structural Markdown parsing.
use std::{borrow::Cow, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub range: Range<usize>,
    pub block: bool,
}

pub fn ranges(source: &str) -> Vec<Comment> {
    scan(source, false)
}

pub fn editing_ranges(source: &str) -> Vec<Comment> {
    scan(source, true)
}

fn scan(source: &str, editing: bool) -> Vec<Comment> {
    use markdown_parser::mdast::Node;
    fn exclude(node: &Node, source: &str, out: &mut Vec<Range<usize>>) {
        if matches!(
            node,
            Node::Code(_)
                | Node::InlineCode(_)
                | Node::Yaml(_)
                | Node::Html(_)
                | Node::Image(_)
                | Node::Definition(_)
                | Node::Math(_)
                | Node::InlineMath(_)
        ) {
            if let Some(p) = node.position() {
                out.push(p.start.offset..p.end.offset);
            }
        } else if let Some(children) = node.children() {
            if matches!(node, Node::Link(_))
                && let Some(position) = node.position()
            {
                let begin = children
                    .last()
                    .and_then(|child| child.position())
                    .map_or(position.start.offset, |p| p.end.offset);
                if let Some(offset) = source[begin..position.end.offset].find("](") {
                    out.push(begin + offset + 2..position.end.offset);
                } else if source.as_bytes().get(position.start.offset) == Some(&b'<') {
                    out.push(position.start.offset..position.end.offset);
                    return;
                }
            }
            for child in children {
                exclude(child, source, out);
            }
        }
    }
    if !source.contains("%%") {
        return vec![];
    }
    let mut blocked = vec![];
    if let Some(root) = crate::syntax::parse_raw(source) {
        exclude(&root, source, &mut blocked);
    }
    blocked.sort_by_key(|range| range.start);
    let mut result = vec![];
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("%%") {
        let start = cursor + found;
        cursor = start + 2;
        if source[..start]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count()
            % 2
            == 1
            || blocked
                .get(blocked.partition_point(|r| r.end <= start))
                .is_some_and(|r| r.start <= start)
        {
            continue;
        }
        let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = source[cursor..]
            .find('\n')
            .map_or(source.len(), |i| cursor + i);
        let block = source[line_start..start]
            .trim_matches([' ', '\t', '>'])
            .is_empty()
            && !source[cursor..line_end].contains('%')
            && line_end < source.len();
        let end = if block || editing {
            source[cursor..]
                .find("%%")
                .map_or(source.len(), |i| cursor + i + 2)
        } else if let Some(close) = source[cursor..line_end].find("%%") {
            cursor + close + 2
        } else {
            continue;
        };
        // Markdown inside a comment must not make later comments look like code.
        let changes_code = block || blocked.iter().any(|r| start <= r.start && r.start < end);
        result.push(Comment {
            range: start..end,
            block,
        });
        if changes_code {
            let clean = masked(source, &result);
            blocked.clear();
            if let Some(root) = crate::syntax::parse_raw(&clean) {
                exclude(&root, &clean, &mut blocked);
            }
            blocked.sort_by_key(|range| range.start);
        }
        cursor = end;
    }
    result
}

/// Toggle comment delimiters for all selections as one source transform.
pub fn toggle(source: &str, selections: &[Range<usize>]) -> Option<(String, Vec<Range<usize>>)> {
    use std::collections::BTreeMap;
    use unicode_segmentation::UnicodeSegmentation;
    if selections.iter().any(|r| {
        r.start > r.end || !source.is_char_boundary(r.start) || !source.is_char_boundary(r.end)
    }) {
        return None;
    }
    let comments = editing_ranges(source);
    let graphemes: Vec<_> = source.grapheme_indices(true).collect();
    let word = |offset: usize| {
        let is_word = |index: usize| {
            graphemes[index]
                .1
                .chars()
                .any(|c| c.is_alphanumeric() || c == '_')
        };
        let next = graphemes.partition_point(|(at, _)| *at < offset);
        let at = if next < graphemes.len() && is_word(next) {
            Some(next)
        } else {
            next.checked_sub(1).filter(|&i| is_word(i))
        };
        let Some(mut start) = at else {
            return offset..offset;
        };
        let mut end = start + 1;
        while start > 0 && is_word(start - 1) {
            start -= 1;
        }
        while end < graphemes.len() && is_word(end) {
            end += 1;
        }
        graphemes[start].0..graphemes.get(end).map_or(source.len(), |(at, _)| *at)
    };
    let mut removals = vec![];
    let mut wraps = vec![];
    let mut plans = vec![];
    for selection in selections {
        if let Some(comment) = comments.iter().find(|comment| {
            comment.range.start <= selection.start && selection.end <= comment.range.end
        }) {
            removals.push(comment.range.start..comment.range.start + 2);
            if comment.range.len() >= 4 && source[comment.range.clone()].ends_with("%%") {
                removals.push(comment.range.end - 2..comment.range.end);
            }
            plans.push((selection.clone(), false));
        } else {
            let range = if selection.is_empty() {
                word(selection.start)
            } else {
                selection.clone()
            };
            for comment in &comments {
                let mut markers: Vec<_> =
                    std::iter::once(comment.range.start..comment.range.start + 2).collect();
                if comment.range.len() >= 4 && source[comment.range.clone()].ends_with("%%") {
                    markers.push(comment.range.end - 2..comment.range.end);
                }
                removals.extend(
                    markers
                        .into_iter()
                        .filter(|marker| marker.start < range.end && marker.end > range.start),
                );
            }
            wraps.push(range.clone());
            plans.push((range, true));
        }
    }
    fn merged(mut ranges: Vec<Range<usize>>, adjacent: bool) -> Vec<Range<usize>> {
        ranges.sort_by_key(|r| (r.start, r.end));
        let mut out: Vec<Range<usize>> = vec![];
        for range in ranges {
            if let Some(last) = out.last_mut()
                && (range.start < last.end || range == *last || adjacent && range.start == last.end)
            {
                last.end = last.end.max(range.end);
            } else {
                out.push(range);
            }
        }
        out
    }
    let removals = merged(removals, true);
    let mut insertions: BTreeMap<usize, String> = BTreeMap::new();
    for range in merged(wraps, false) {
        insertions.entry(range.start).or_default().push_str("%%");
        insertions.entry(range.end).or_default().push_str("%%");
    }
    let mapped = |offset: usize| {
        offset
            - removals
                .iter()
                .map(|r| offset.saturating_sub(r.start).min(r.len()))
                .sum::<usize>()
            + insertions
                .range(..offset)
                .map(|(_, text)| text.len())
                .sum::<usize>()
    };
    let selections = plans
        .into_iter()
        .map(|(range, wrapped)| {
            if !wrapped {
                mapped(range.start)..mapped(range.end)
            } else if range.is_empty() {
                let at = mapped(range.start) + 2;
                at..at
            } else {
                mapped(range.start) + insertions.get(&range.start).map_or(0, String::len)
                    ..mapped(range.end)
            }
        })
        .collect();
    let mut text = String::with_capacity(source.len() + insertions.len() * 2);
    let mut removed = 0;
    for (offset, ch) in source.char_indices() {
        if let Some(inserted) = insertions.get(&offset) {
            text.push_str(inserted);
        }
        while removed < removals.len() && removals[removed].end <= offset {
            removed += 1;
        }
        if removals.get(removed).is_none_or(|r| r.start > offset) {
            text.push(ch);
        }
    }
    if let Some(inserted) = insertions.get(&source.len()) {
        text.push_str(inserted);
    }
    Some((text, selections))
}

pub fn masked<'a>(source: &'a str, comments: &[Comment]) -> Cow<'a, str> {
    if comments.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut bytes = source.as_bytes().to_vec();
    for comment in comments {
        for byte in &mut bytes[comment.range.clone()] {
            if !matches!(*byte, b'\r' | b'\n') {
                *byte = if comment.block { b' ' } else { 1 };
            }
        }
    }
    Cow::Owned(
        String::from_utf8(bytes).expect("comments are replaced with ASCII at UTF-8 boundaries"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn toggle_comments_words_empty_ranges_and_multicursor() {
        let (text, selected) = toggle("one 中文", &[1..1]).unwrap();
        assert_eq!(text, "%%one%% 中文");
        assert_eq!(selected, [2..5]);
        assert_eq!(toggle(&text, &selected).unwrap().0, "one 中文");
        let (text, selected) = toggle(" ", &[0..0]).unwrap();
        assert_eq!(text, "%%%% ");
        assert_eq!(selected, [2..2]);
        assert_eq!(
            toggle(&text, &selected).unwrap(),
            (" ".into(), std::iter::once(0..0).collect())
        );
        let (text, selected) = toggle("one two", &[0..3, 4..7]).unwrap();
        assert_eq!(text, "%%one%% %%two%%");
        assert_eq!(selected, [2..5, 10..13]);
        assert_eq!(toggle(&text, &selected).unwrap().0, "one two");
        assert_eq!(
            toggle("%%alpha beta%%", &[2..7, 8..12]).unwrap(),
            ("alpha beta".into(), vec![0..5, 6..10])
        );
        assert_eq!(toggle("a %%b%% c", &[0..9]).unwrap().0, "%%a b c%%");
        assert_eq!(toggle("one\r\ntwo", &[0..8]).unwrap().0, "%%one\r\ntwo%%");
        assert_eq!(toggle("e\u{301}中", &[3..3]).unwrap().0, "%%e\u{301}中%%");
        assert_eq!(
            toggle("prefix %%unfinished", &[12..12]).unwrap().0,
            "prefix unfinished"
        );
        assert!(toggle("中", &[1..1]).is_none());
    }
    #[test]
    fn inline_block_code_escape_and_byte_positions() {
        let source = "---\ntitle: '%%yaml%%'\n---\nA %%中文%% B `%%code%%` \\%%literal\n\n%%\n```\n[[hidden]]\n%%\nvisible %%tail%%";
        let found = ranges(source);
        assert_eq!(found.len(), 3);
        assert!(!found[0].block && found[1].block && !found[2].block);
        assert_eq!(&source[found[0].range.clone()], "%%中文%%");
        let masked = masked(source, &found);
        assert_eq!(masked.len(), source.len());
        assert_eq!(masked.find("visible"), source.find("visible"));
        assert!(masked.contains("%%code%%"));
        assert_eq!(ranges("text %%unfinished").len(), 0);
        assert_eq!(ranges("%%\nunclosed")[0].range, 0.."%%\nunclosed".len());
        assert!(ranges("[link](https://example.com/%%value%%) ![img](%%file%%.png)").is_empty());
        assert_eq!(ranges("%%\n```\n%%\n```\n%%code%%\n```").len(), 1);
    }
    #[test]
    fn comments_do_not_contribute_structure_or_rendered_content() {
        let source = "# Visible %%中文secret%% Title\n\n%%\n# Hidden\n[[Ghost]]\n- [ ] ghost\n%%\n\n[[Kept]] and `%%code%%`\n- [ ] actual";
        let parsed = crate::index::parse(source);
        assert_eq!(parsed.comments.len(), 2);
        assert_eq!(parsed.headings.len(), 1);
        assert_eq!(parsed.headings[0].title, "Visible  Title");
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].target, "Kept");
        let mut index = crate::index::Index::default();
        index.update("a.md".into(), source.into());
        assert!(index.search("task:ghost").is_empty());
        assert_eq!(index.search("task:actual").len(), 1);
        assert!(!index.search("secret").is_empty());
        assert!(!index.search("section:(Visible Kept)").is_empty());
        let reading = crate::rendering::reading_document(
            &crate::index::Index::default(),
            std::path::Path::new("a.md"),
            source,
        );
        assert!(!reading.markdown.contains("secret"));
        assert!(!reading.markdown.contains("Ghost"));
        assert!(!reading.markdown.contains('\u{1}'));
        assert!(!reading.markdown.contains('\u{2060}'));
        assert!(reading.markdown.contains("%%code%%"));
        assert_eq!(reading.tasks.len(), 1);
        let marker = source.find("[ ] actual").unwrap() + 1;
        assert_eq!(reading.tasks[0].marker, marker..marker + 1);
        assert!(
            crate::index::set_task(source, reading.tasks[0].marker.clone(), true)
                .unwrap()
                .ends_with("- [x] actual")
        );
        let styles = crate::markdown::spans(source);
        assert_eq!(
            styles
                .iter()
                .filter(|span| span.kind == crate::markdown::Kind::Comment)
                .count(),
            2
        );
    }
}
