//! Source-backed candidates for transient live presentation.
use crate::syntax::Snapshot;
use markdown_parser::mdast::Node;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub source: Range<usize>,
    pub block: bool,
}

pub fn candidates(snapshot: &Snapshot) -> Vec<Candidate> {
    fn walk(node: &Node, source: &str, out: &mut Vec<Candidate>) {
        let raw = node
            .position()
            .and_then(|p| source.get(p.start.offset..p.end.offset))
            .unwrap_or_default();
        let block = match node {
            Node::Math(_) | Node::Table(_) => Some(true),
            Node::InlineMath(_) | Node::Image(_) => Some(false),
            Node::Code(n)
                if n.lang
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) =>
            {
                Some(true)
            }
            Node::Blockquote(_)
                if raw
                    .lines()
                    .next()
                    .is_some_and(|l| l.trim_start_matches([' ', '\t', '>']).starts_with("[!")) =>
            {
                Some(true)
            }
            Node::Paragraph(p)
                if p.children
                    .iter()
                    .any(|n| matches!(n, Node::Html(h) if !h.value.starts_with("<!--"))) =>
            {
                Some(true)
            }
            Node::Html(h) if !h.value.trim_start().starts_with("<!--") => Some(true),
            _ => None,
        };
        if let Some(block) = block {
            if let Some(p) = node.position() {
                out.push(Candidate {
                    source: p.start.offset..p.end.offset,
                    block,
                });
            }
            return;
        }
        if matches!(
            node,
            Node::Code(_) | Node::InlineCode(_) | Node::Yaml(_) | Node::Html(_)
        ) {
            return;
        }
        if let Some(children) = node.children() {
            for n in children {
                walk(n, source, out);
            }
        }
    }
    let mut out = vec![];
    if let Some(ast) = snapshot.ast.as_deref() {
        walk(ast, &snapshot.source, &mut out);
    }
    for link in crate::index::parse_snapshot(snapshot).links {
        if link.range.start > 0
            && snapshot.source.as_bytes()[link.range.start - 1] == b'!'
            && snapshot.source[..link.range.start - 1]
                .bytes()
                .rev()
                .take_while(|b| *b == b'\\')
                .count()
                % 2
                == 0
        {
            out.push(Candidate {
                source: link.range.start - 1..link.range.end,
                block: !crate::rendering::attachment_target(&link.target),
            });
        }
    }
    out.sort_by_key(|c| (c.source.start, std::cmp::Reverse(c.source.end)));
    let mut end = 0;
    out.retain(|c| {
        if c.source.start < end {
            false
        } else {
            end = c.source.end;
            true
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_candidates_keep_source_ranges_and_choose_outer_callouts() {
        let source = "text $x^2$\n\n> [!note] title\n> ![inner](x.png)\n> $y$\n\n| a |\n| --- |\n| b |\n\n![[other]]\n\n`![code](x)`";
        let out = candidates(&Snapshot::new(source));
        assert_eq!(out.len(), 4, "{out:?}");
        assert_eq!(&source[out[0].source.clone()], "$x^2$");
        assert!(source[out[1].source.clone()].starts_with("> [!note]"));
        assert!(out[1].block);
        assert_eq!(&source[out[3].source.clone()], "![[other]]");
        assert!(
            out.windows(2)
                .all(|pair| pair[0].source.end <= pair[1].source.start)
        );
    }
}
