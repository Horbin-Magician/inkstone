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
    candidates_with_parsed(snapshot, &crate::index::parse_snapshot(snapshot))
}

fn candidates_with_parsed(
    snapshot: &Snapshot,
    parsed: &crate::index::ParsedNote,
) -> Vec<Candidate> {
    fn walk(node: &Node, source: &str, out: &mut Vec<Candidate>) {
        let raw = node
            .position()
            .and_then(|p| source.get(p.start.offset..p.end.offset))
            .unwrap_or_default();
        let block = match node {
            Node::Math(_) | Node::Table(_) => Some(true),
            Node::InlineMath(_) | Node::Image(_) | Node::ImageReference(_) => Some(false),
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
    for link in &parsed.links {
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

// Index once: looking up each formula by walking the entire AST makes opening
// a note quadratic in its formula count. Exact ranges exclude nested graphics
// owned by a table, callout, or footnote footer.
fn graphics(node: &Node, out: &mut BTreeMap<(usize, usize), (crate::graphics::Kind, String)>) {
    use crate::graphics::Kind;
    let graphic = match node {
        Node::InlineMath(n) => Some((Kind::InlineMath, &n.value)),
        Node::Math(n) => Some((Kind::BlockMath, &n.value)),
        Node::Code(n)
            if n.lang
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) =>
        {
            Some((Kind::Mermaid, &n.value))
        }
        _ => None,
    };
    if let Some((kind, value)) = graphic
        && let Some(p) = node.position()
    {
        out.insert((p.start.offset, p.end.offset), (kind, value.clone()));
    }
    if let Some(children) = node.children() {
        for child in children {
            graphics(child, out);
        }
    }
}

pub fn fragments(
    index: &crate::index::Index,
    path: &Path,
    snapshot: Arc<Snapshot>,
    reading: &crate::rendering::ReadingDocument,
) -> Vec<Fragment> {
    fragments_cancellable(index, path, snapshot, reading, || false).unwrap()
}

/// Cancel between preparation stages and individual fragments. Never return a
/// partial projection; an individual parser/render call is not preempted.
pub fn fragments_cancellable(
    index: &crate::index::Index,
    path: &Path,
    snapshot: Arc<Snapshot>,
    reading: &crate::rendering::ReadingDocument,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<Fragment>> {
    if cancelled() {
        return None;
    }
    let parsed = crate::index::parse_snapshot(&snapshot);
    let mut out = candidates_with_parsed(&snapshot, &parsed);
    let mut graphic_nodes = BTreeMap::new();
    if let Some(ast) = snapshot.ast.as_deref() {
        graphics(ast, &mut graphic_nodes);
    }
    if cancelled() {
        return None;
    }
    let numbers = reading.footnote_numbers();
    if cancelled() {
        return None;
    }
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
    let fragments = out
        .into_iter()
        .map(|candidate| {
            if cancelled() {
                return None;
            }
            let graphic = (candidate.role == Role::Content)
                .then(|| graphic_nodes.remove(&(candidate.source.start, candidate.source.end)))
                .flatten();
            if graphic.is_some() {
                return Some(Fragment {
                    document: crate::rendering::ReadingDocument::graphic_fragment(
                        path,
                        snapshot.source.clone(),
                        candidate.source.clone(),
                    ),
                    candidate,
                    numbers: BTreeMap::new(),
                    targets: vec![],
                    graphic,
                });
            }
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
            Some(Fragment {
                candidate,
                document,
                numbers: overrides,
                targets,
                graphic: None,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!cancelled()).then_some(fragments)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_never_returns_partial_fragments() {
        use std::cell::Cell;
        let source = "$a$ and $b$ and $c$\n\n| a | b |\n| - | - |\n| 中 | 😀 |";
        let path = Path::new("note.md");
        let index = crate::index::Index::default();
        let snapshot = Arc::new(Snapshot::new(source));
        let reading =
            crate::rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let expected = fragments(&index, path, snapshot.clone(), &reading);
        assert_eq!(expected.len(), 4);
        for stop in 0..=7 {
            let calls = Cell::new(0);
            let result = fragments_cancellable(&index, path, snapshot.clone(), &reading, || {
                let call = calls.get();
                calls.set(call + 1);
                call >= stop
            });
            assert!(result.is_none(), "cancel checkpoint {stop}");
        }
        let result = fragments_cancellable(&index, path, snapshot, &reading, || false).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|f| (&f.candidate, &f.document.markdown))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|f| (&f.candidate, &f.document.markdown))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn graphic_fragments_preserve_literal_content_and_source_mapping() {
        let source = format!(
            "{}\n\n> 引用 $z$\n\n> $$\n> a+b\n> $$\n\n```mermaid\nflowchart LR\nA[\"[[note]] ==text== [^n]\"] --> B\n```\n\n尾部[^n]\n\n[^n]: 正文",
            include_str!("../tests/fixtures/markdown/math.md")
        );
        let path = Path::new("folder/formulas.md");
        let index = crate::index::Index::default();
        let snapshot = Arc::new(Snapshot::new(&source));
        let reading =
            crate::rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let fragments = fragments(&index, path, snapshot.clone(), &reading);
        let graphics: Vec<_> = fragments.iter().filter(|f| f.graphic.is_some()).collect();
        assert!(graphics.len() > 10);
        assert!(
            graphics
                .iter()
                .any(|f| { f.graphic.as_ref().unwrap().0 == crate::graphics::Kind::Mermaid })
        );
        for fragment in graphics {
            let range = fragment.candidate.source.clone();
            let original =
                crate::rendering::reading_snapshot(&index, path, snapshot.clone(), range.clone());
            assert_eq!(fragment.document.markdown, original.markdown);
            assert!(fragment.document.source_matches(path, &source));
            assert!(fragment.document.references.is_empty());
            assert!(fragment.document.tasks.is_empty());
            assert!(fragment.numbers.is_empty());
            for (offset, _) in source[range.clone()].char_indices() {
                assert_eq!(
                    fragment.document.output_offset(path, range.start + offset),
                    original.output_offset(path, range.start + offset),
                );
            }
        }
        assert!(
            fragments
                .iter()
                .any(|f| matches!(f.candidate.role, Role::Footer(_)))
        );
    }

    #[test]
    fn double_dollars_are_block_math_with_original_source_ranges() {
        let source = "中文 $$x^2$$ 后文 $y$\r\n\r\n$$\\frac{a}{b}$$\r\n\r\n> $$z$$\r\n\r\n- $$w$$\r\n\r\n$$\r\nu+v\r\n$$\r\n\r\n`$$code$$`\r\n\r\n```text\r\n$$code$$\r\n```\r\n\r\n\\$\\$escaped\\$\\$\r\n\r\n**$$bold$$**";
        let snapshot = Arc::new(Snapshot::new(source));
        let path = Path::new("note.md");
        let index = crate::index::Index::default();
        let reading =
            crate::rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let fragments = fragments(&index, path, snapshot, &reading);
        let math: Vec<_> = fragments.iter().filter(|f| f.graphic.is_some()).collect();
        assert_eq!(math.len(), 7);
        for fragment in math {
            let raw = &source[fragment.candidate.source.clone()];
            let block = raw.starts_with("$$");
            assert_eq!(fragment.candidate.block, block, "{raw}");
            assert_eq!(
                fragment.graphic.as_ref().unwrap().0,
                if block {
                    crate::graphics::Kind::BlockMath
                } else {
                    crate::graphics::Kind::InlineMath
                }
            );
            assert!(fragment.document.source_matches(path, source));
        }
    }

    #[test]
    fn nested_graphics_stay_owned_by_their_outer_fragment() {
        let source = "$x$\n\n> [!note]\n> $y$\n\n| A |\n| --- |\n| $z$ |\n\n尾部";
        let path = Path::new("note.md");
        let index = crate::index::Index::default();
        let snapshot = Arc::new(Snapshot::new(source));
        let reading =
            crate::rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let fragments = fragments(&index, path, snapshot, &reading);
        assert_eq!(fragments.len(), 3);
        assert_eq!(
            fragments[0].graphic,
            Some((crate::graphics::Kind::InlineMath, "x".into()))
        );
        assert!(fragments[1..].iter().all(|f| f.graphic.is_none()));
        assert!(fragments[1].document.markdown.contains("$y$"));
        assert!(fragments[2].document.markdown.contains("$z$"));
    }

    #[test]
    fn reference_images_render_in_isolated_fragments_and_keep_origin_and_title() {
        let source = "中文 ![替代😀][PIC]\r\n\r\n![pic][]\r\n\r\n![pic]\r\n\r\n`![pic]`\r\n\r\n[pic]: assets/image.svg \"图片标题\"\r\n[pic]: ignored.svg\r\n";
        let path = Path::new("folder/note.md");
        let index = crate::index::Index::default();
        let snapshot = Arc::new(Snapshot::new(source));
        let reading =
            crate::rendering::reading_snapshot(&index, path, snapshot.clone(), 0..source.len());
        let fragments = fragments(&index, path, snapshot, &reading);
        assert_eq!(fragments.len(), 3);
        for fragment in fragments {
            assert!(!fragment.candidate.block);
            assert_eq!(fragment.document.references.len(), 1);
            assert_eq!(fragment.document.references[0].target, "assets/image.svg");
            assert_eq!(fragment.document.references[0].from, path);
            assert!(fragment.document.markdown.contains("\"图片标题\""));
            assert_eq!(
                fragment
                    .document
                    .output_offset(path, fragment.candidate.source.start),
                Some(0)
            );
            let parsed = Snapshot::new(&fragment.document.markdown);
            assert!(matches!(
                parsed.ast.as_deref().unwrap().children().unwrap()[0]
                    .children()
                    .unwrap()[0],
                Node::Image(_)
            ));
            assert!(fragment.document.source_matches(path, source));
        }
        assert!(reading.markdown.contains("`![pic]`"));
    }

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
