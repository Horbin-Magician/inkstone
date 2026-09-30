//! Obsidian comments, with byte-preserving input for structural Markdown parsing.
use std::{borrow::Cow, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub range: Range<usize>,
    pub block: bool,
}

pub fn ranges(source: &str) -> Vec<Comment> {
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
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let mut blocked = vec![];
    if let Ok(root) = markdown_parser::to_mdast(source, &options) {
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
        let end = if block {
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
            if let Ok(root) = markdown_parser::to_mdast(&clean, &options) {
                exclude(&root, &clean, &mut blocked);
            }
            blocked.sort_by_key(|range| range.start);
        }
        cursor = end;
    }
    result
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
