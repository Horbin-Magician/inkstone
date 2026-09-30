//! Source-backed candidates for transient live presentation.
use crate::syntax::Snapshot;
use markdown_parser::mdast::Node;
use std::ops::Range;
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Content,
    Reference,
    Footer(Vec<Range<usize>>),
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub source: Range<usize>,
    pub block: bool,
    pub role: Role,
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
                    role: Role::Content,
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
                role: Role::Content,
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

pub struct Fragment {
    pub candidate: Candidate,
    pub document: crate::rendering::ReadingDocument,
    pub numbers: BTreeMap<String, usize>,
    pub targets: Vec<(usize, std::path::PathBuf, usize)>,
    pub graphic: Option<(crate::graphics::Kind, String)>,
}

pub fn fragments(
    index: &crate::index::Index,
    path: &Path,
    snapshot: Arc<Snapshot>,
    reading: &crate::rendering::ReadingDocument,
) -> Vec<Fragment> {
    let mut out = candidates(&snapshot);
    let numbers = reading.footnote_numbers();
    let parsed = crate::index::parse_snapshot(&snapshot);
    let refs = parsed
        .footnote_references
        .iter()
        .map(|(r, _)| r.start - 2..r.end + 1)
        .chain(parsed.inline_footnotes.iter().map(|n| n.range.clone()));
    let mut labels = BTreeMap::new();
    for range in refs {
        if let Some(number) = numbers.get(&(path.to_path_buf(), range.start)) {
            labels.insert(range.start, *number);
            out.push(Candidate {
                source: range,
                block: false,
                role: Role::Reference,
            });
        }
    }
    if !numbers.is_empty() {
        let tail = snapshot
            .ast
            .as_deref()
            .and_then(Node::children)
            .and_then(|nodes| {
                nodes
                    .iter()
                    .rev()
                    .find(|n| !matches!(n, Node::FootnoteDefinition(_) | Node::Yaml(_)))
            })
            .and_then(Node::position);
        if let Some(tail) = tail {
            let source = tail.start.offset..tail.end.offset;
            out.retain(|c| !(c.source.start < source.end && source.start < c.source.end));
            let display = Snapshot::new(&reading.markdown);
            let mut roots: Vec<_> = display
                .ast
                .as_deref()
                .and_then(Node::children)
                .into_iter()
                .flatten()
                .filter(|n| !matches!(n, Node::FootnoteDefinition(_)))
                .filter_map(Node::position)
                .map(|p| p.start.offset..p.end.offset)
                .collect();
            roots.pop();
            out.push(Candidate {
                source,
                block: true,
                role: Role::Footer(roots),
            });
        }
        for (range, _) in parsed.footnote_definitions {
            out.push(Candidate {
                source: range,
                block: true,
                role: Role::Hidden,
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
    out.into_iter()
        .map(|candidate| {
            let document = match candidate.role {
                Role::Footer(_) => reading.clone(),
                Role::Reference | Role::Hidden => {
                    let mut document = reading.clone();
                    document.markdown = if candidate.role == Role::Reference {
                        format!("<sup>{}</sup>", labels[&candidate.source.start])
                    } else {
                        String::new()
                    };
                    document.tasks.clear();
                    document
                }
                Role::Content => crate::rendering::reading_snapshot(
                    index,
                    path,
                    snapshot.clone(),
                    candidate.source.clone(),
                ),
            };
            let overrides = document.footnote_overrides(&numbers);
            let targets = if matches!(candidate.role, Role::Footer(_)) {
                document.footnote_definition_targets()
            } else {
                vec![]
            };
            fn graphic(
                node: &Node,
                range: &Range<usize>,
            ) -> Option<(crate::graphics::Kind, String)> {
                if node
                    .position()
                    .is_some_and(|p| p.start.offset == range.start && p.end.offset == range.end)
                {
                    match node {
                        Node::InlineMath(n) => {
                            return Some((crate::graphics::Kind::InlineMath, n.value.clone()));
                        }
                        Node::Math(n) => {
                            return Some((crate::graphics::Kind::BlockMath, n.value.clone()));
                        }
                        Node::Code(n)
                            if n.lang
                                .as_deref()
                                .is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) =>
                        {
                            return Some((crate::graphics::Kind::Mermaid, n.value.clone()));
                        }
                        _ => {}
                    }
                }
                node.children()
                    .and_then(|children| children.iter().find_map(|n| graphic(n, range)))
            }
            let graphic = if candidate.role == Role::Content {
                snapshot
                    .ast
                    .as_deref()
                    .and_then(|ast| graphic(ast, &candidate.source))
            } else {
                None
            };
            Fragment {
                candidate,
                document,
                numbers: overrides,
                targets,
                graphic,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn footnote_fragments_number_refs_and_place_all_definitions_in_one_footer() {
        let source = "# top\n\nA^[短] B[^named] C^[中文 **粗体**]\n\n尾部\n\n[^named]: 命名定义";
        let path = Path::new("note.md");
        let snapshot = Arc::new(Snapshot::new(source));
        let reading = crate::rendering::reading_snapshot(
            &crate::index::Index::default(),
            path,
            snapshot.clone(),
            0..source.len(),
        );
        let numbers = reading.footnote_numbers();
        assert_eq!(
            numbers[&(path.to_path_buf(), source.find("^[短]").unwrap())],
            1
        );
        assert_eq!(
            numbers[&(path.to_path_buf(), source.find("[^named]").unwrap())],
            2
        );
        assert_eq!(
            numbers[&(path.to_path_buf(), source.find("^[中文").unwrap())],
            3
        );
        let fragments = fragments(&crate::index::Index::default(), path, snapshot, &reading);
        assert_eq!(
            fragments
                .iter()
                .filter(|f| f.candidate.role == Role::Reference)
                .count(),
            3
        );
        assert_eq!(
            fragments
                .iter()
                .filter(|f| matches!(f.candidate.role, Role::Footer(_)))
                .count(),
            1
        );
        assert_eq!(
            fragments
                .iter()
                .filter(|f| f.candidate.role == Role::Hidden)
                .count(),
            1
        );
    }
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
