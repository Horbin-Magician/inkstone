//! Source-based Markdown metadata, fold regions, anchors, and task edits.

use markdown_parser::mdast::Node;
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WikiLink {
    pub range: Range<usize>,
    pub target: String,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub offset: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskItem {
    pub start: usize,
    pub marker: Range<usize>,
    pub checked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockReference {
    pub id: String,
    pub range: Range<usize>,
    pub marker: Range<usize>,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ParsedNote {
    pub inline_footnotes: Vec<crate::syntax::InlineFootnote>,
    pub comments: Vec<crate::comments::Comment>,
    pub footnotes: Vec<(Range<usize>, String)>,
    pub footnote_references: Vec<(Range<usize>, String)>,
    pub footnote_definitions: Vec<(Range<usize>, String)>,
    pub links: Vec<WikiLink>,
    pub headings: Vec<Heading>,
    pub tags: Vec<String>,
    pub tag_counts: BTreeMap<String, usize>,
    pub inline_tags: Vec<(Range<usize>, String)>,
    pub aliases: Vec<String>,
    pub standard_links: Vec<(String, usize)>,
    pub destinations: Vec<(Range<usize>, String)>,
    pub tasks: Vec<TaskItem>,
    pub blocks: Vec<BlockReference>,
    /// Zero-based header row through the exclusive body end row.
    pub folds: Vec<Range<usize>>,
    pub fold_regions: Vec<(Range<usize>, FoldKind)>,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum FoldKind {
    Heading,
    Indent,
    Block,
}

impl ParsedNote {
    pub fn fold_ranges(&self, headings: bool, indentation: bool) -> Vec<Range<usize>> {
        let mut ranges: Vec<_> = self
            .fold_regions
            .iter()
            .filter(|(_, kind)| match kind {
                FoldKind::Heading => headings,
                FoldKind::Indent => indentation,
                FoldKind::Block => headings || indentation,
            })
            .map(|(range, _)| range.clone())
            .collect();
        ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
        ranges.dedup_by_key(|r| r.start);
        ranges
    }
}

pub fn parse(source: &str) -> ParsedNote {
    parse_snapshot(&crate::syntax::Snapshot::new(source))
}

pub fn parse_snapshot(snapshot: &crate::syntax::Snapshot) -> ParsedNote {
    let source = snapshot.structural.as_ref();
    fn label(node: &Node) -> String {
        match node {
            Node::Text(n) => n.value.replace('\u{1}', ""),
            Node::InlineCode(n) => n.value.clone(),
            _ => node
                .children()
                .map(|c| c.iter().map(label).collect())
                .unwrap_or_default(),
        }
    }
    fn walk(node: &Node, source: &str, result: &mut ParsedNote) {
        let footnote = match node {
            Node::FootnoteReference(n) => Some((&n.identifier, &n.position)),
            Node::FootnoteDefinition(n) => Some((&n.identifier, &n.position)),
            _ => None,
        };
        if let Node::FootnoteDefinition(n) = node
            && let Some(p) = &n.position
        {
            result
                .footnote_definitions
                .push((p.start.offset..p.end.offset, n.identifier.to_lowercase()));
        }
        if let Some((identifier, Some(position))) = footnote {
            let raw = &source[position.start.offset..position.end.offset];
            if let Some(label) = raw.strip_prefix("[^") {
                let mut escaped = false;
                for (i, ch) in label.char_indices() {
                    if ch == ']' && !escaped {
                        if matches!(node, Node::FootnoteReference(_)) {
                            result.footnote_references.push((
                                position.start.offset + 2..position.start.offset + 2 + i,
                                identifier.to_lowercase(),
                            ));
                        }
                        result.footnotes.push((
                            position.start.offset + 2..position.start.offset + 2 + i,
                            identifier.to_lowercase(),
                        ));
                        break;
                    }
                    if ch == '\\' {
                        escaped = !escaped;
                    } else {
                        escaped = false;
                    }
                }
            }
        }
        if let Node::ListItem(item) = node
            && let Some(position) = &item.position
        {
            let raw = &source[position.start.offset..position.end.offset];
            if let Some((marker, checked)) = task_marker(raw) {
                result.tasks.push(TaskItem {
                    start: position.start.offset,
                    marker: position.start.offset + marker.start
                        ..position.start.offset + marker.end,
                    checked,
                });
            }
        }
        let destination = match node {
            Node::Link(n) => Some((&n.url, &n.position)),
            Node::Image(n) => Some((&n.url, &n.position)),
            Node::Definition(n) => Some((&n.url, &n.position)),
            _ => None,
        };
        if let Some((url, Some(position))) = destination
            && !url.is_empty()
        {
            let raw = &source[position.start.offset..position.end.offset];
            if let Some(start) = raw.rfind("](").or_else(|| raw.find("]:"))
                && let Some(offset) = raw[start + 2..].find(url.as_str())
            {
                let offset = position.start.offset + start + 2 + offset;
                result
                    .destinations
                    .push((offset..offset + url.len(), url.clone()));
            }
        }
        if let Node::Link(link) = node
            && let Some(position) = &link.position
        {
            result
                .standard_links
                .push((link.url.clone(), position.start.offset));
        }
        if let Node::Yaml(yaml) = node {
            let (tags, aliases) = crate::properties::metadata(&yaml.value);
            result.tags.extend(tags);
            result.aliases.extend(aliases);
        }
        if let Node::Heading(heading) = node
            && let Some(position) = &heading.position
        {
            result.headings.push(Heading {
                level: heading.depth,
                title: label(node),
                offset: position.start.offset,
            });
        }
        if let Node::Text(text) = node
            && let Some(position) = &text.position
        {
            let range = position.start.offset..position.end.offset;
            if let Some(raw) = source.get(range.clone()) {
                for (i, ch) in raw.char_indices().filter(|(_, ch)| *ch == '#') {
                    let _ = ch;
                    if range.start + i > 0
                        && source[..range.start + i]
                            .chars()
                            .next_back()
                            .is_some_and(|c| {
                                c.is_alphanumeric() || matches!(c, '\\' | '/' | '[' | '#')
                            })
                    {
                        continue;
                    }
                    let tag: String = raw[i + 1..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                        .collect();
                    if !tag.is_empty() && tag.chars().any(|c| !c.is_numeric()) {
                        result.inline_tags.push((
                            range.start + i + 1..range.start + i + 1 + tag.len(),
                            tag.clone(),
                        ));
                        result.tags.push(tag);
                    }
                }
                let mut offset = 0;
                while let Some(start) = raw[offset..].find("[[") {
                    let start = offset + start;
                    let preceding = source[..range.start + start]
                        .bytes()
                        .rev()
                        .take_while(|b| *b == b'\\')
                        .count();
                    if preceding % 2 == 1 {
                        offset = start + 2;
                        continue;
                    }
                    let Some(end) = raw[start + 2..].find("]]").map(|n| start + 2 + n) else {
                        break;
                    };
                    let inner = &raw[start + 2..end];
                    if !inner.contains(['\r', '\n']) {
                        let (target, label) = inner.split_once('|').unwrap_or((inner, inner));
                        let target = target.replace('\u{1}', "");
                        if !target.trim().is_empty() {
                            result.links.push(WikiLink {
                                range: range.start + start..range.start + end + 2,
                                target: target.trim().into(),
                                label: label.replace('\u{1}', ""),
                            });
                        }
                    }
                    offset = end + 2;
                }
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, result);
            }
        }
    }
    let mut result = ParsedNote {
        inline_footnotes: snapshot.inline_footnotes.clone(),
        comments: snapshot.comments.clone(),
        ..Default::default()
    };
    if let Some(root) = snapshot.ast.as_deref() {
        walk(root, source, &mut result);
        fn folds(node: &Node, out: &mut Vec<(Range<usize>, FoldKind)>) {
            if matches!(
                node,
                Node::ListItem(_)
                    | Node::Blockquote(_)
                    | Node::Code(_)
                    | Node::Yaml(_)
                    | Node::Math(_)
            ) && let Some(p) = node.position()
                && p.end.line > p.start.line
            {
                out.push((
                    p.start.line - 1..p.end.line,
                    if matches!(node, Node::ListItem(_) | Node::Blockquote(_)) {
                        FoldKind::Indent
                    } else {
                        FoldKind::Block
                    },
                ));
            }
            if let Some(children) = node.children() {
                for (i, child) in children.iter().enumerate() {
                    if let Node::Heading(heading) = child
                        && let Some(p) = child.position()
                    {
                        let end = children[i + 1..]
                            .iter()
                            .find_map(|n| match n {
                                Node::Heading(h) if h.depth <= heading.depth => {
                                    n.position().map(|p| p.start.line - 1)
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| node.position().map_or(p.end.line, |p| p.end.line));
                        if end > p.start.line {
                            out.push((p.start.line - 1..end, FoldKind::Heading));
                        }
                    }
                    folds(child, out);
                }
            }
        }
        folds(root, &mut result.fold_regions);
        result.folds = result.fold_ranges(true, true);
        fn collect_blocks(
            node: &Node,
            source: &str,
            parent: Option<Range<usize>>,
            out: &mut Vec<BlockReference>,
        ) {
            if matches!(
                node,
                Node::Code(_)
                    | Node::InlineCode(_)
                    | Node::Yaml(_)
                    | Node::Math(_)
                    | Node::InlineMath(_)
            ) {
                return;
            }
            let position = node.position().map(|p| p.start.offset..p.end.offset);
            let parent = if matches!(node, Node::ListItem(_)) {
                position.clone()
            } else {
                parent
            };
            if matches!(node, Node::Paragraph(_) | Node::Heading(_))
                && let Some(range) = &position
            {
                let raw = source[range.clone()].trim_end();
                if let Some(i) = raw.rfind('^') {
                    let id = &raw[i + 1..];
                    if !id.is_empty()
                        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        && (i == 0 || raw[..i].ends_with(char::is_whitespace))
                    {
                        out.push(BlockReference {
                            id: id.into(),
                            range: parent.clone().unwrap_or_else(|| range.clone()),
                            marker: range.start + i..range.start + raw.len(),
                        });
                    }
                }
            }
            if let Some(children) = node.children() {
                let mut previous: Option<Range<usize>> = None;
                for child in children {
                    let before = out.len();
                    collect_blocks(child, source, parent.clone(), out);
                    if let Some(block) = out.get_mut(before)
                        && block.range.start == block.marker.start
                        && let Some(previous) = &previous
                    {
                        block.range = previous.clone();
                    }
                    if let Some(p) = child.position() {
                        previous = Some(p.start.offset..p.end.offset);
                    }
                }
            }
        }
        collect_blocks(root, source, None, &mut result.blocks);
        fn definitions(node: &Node, out: &mut BTreeMap<String, String>) {
            if let Node::Definition(d) = node {
                out.entry(d.identifier.to_lowercase())
                    .or_insert_with(|| d.url.clone());
            }
            if let Some(children) = node.children() {
                for child in children {
                    definitions(child, out);
                }
            }
        }
        fn references(
            node: &Node,
            defs: &BTreeMap<String, String>,
            out: &mut Vec<(String, usize)>,
        ) {
            if let Node::LinkReference(link) = node
                && let (Some(url), Some(p)) =
                    (defs.get(&link.identifier.to_lowercase()), &link.position)
            {
                out.push((url.clone(), p.start.offset));
            }
            if let Some(children) = node.children() {
                for child in children {
                    references(child, defs, out);
                }
            }
        }
        let mut defs = BTreeMap::new();
        definitions(root, &mut defs);
        references(root, &defs, &mut result.standard_links);
    }
    for tag in &result.tags {
        *result.tag_counts.entry(tag.clone()).or_default() += 1;
    }
    result.tags.sort();
    result.tags.dedup();
    result
}

pub fn anchor_range(source: &str, parsed: &ParsedNote, fragment: &str) -> Option<Range<usize>> {
    let fragment = percent_encoding::percent_decode_str(fragment.trim_start_matches('#'))
        .decode_utf8()
        .ok()?;
    if let Some(id) = fragment.strip_prefix('^') {
        return parsed
            .blocks
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.range.clone());
    }
    fn normalized(value: &str) -> String {
        value
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let i = parsed
        .headings
        .iter()
        .position(|h| h.title == fragment)
        .or_else(|| {
            parsed
                .headings
                .iter()
                .position(|h| normalized(&h.title) == normalized(&fragment))
        })?;
    let heading = &parsed.headings[i];
    let end = parsed.headings[i + 1..]
        .iter()
        .find(|h| h.level <= heading.level)
        .map_or(source.len(), |h| h.offset);
    Some(heading.offset..end)
}

pub(crate) fn task_box_marker(text: &str) -> Option<(Range<usize>, bool)> {
    let mut chars = text.chars();
    if chars.next()? != '[' {
        return None;
    }
    let status = chars.next()?;
    if matches!(status, '\r' | '\n')
        || chars.next()? != ']'
        || chars.next().is_some_and(|ch| !ch.is_whitespace())
    {
        return None;
    }
    Some((1..1 + status.len_utf8(), status != ' '))
}

pub(crate) fn task_marker(raw: &str) -> Option<(Range<usize>, bool)> {
    static PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[ \t]*(?:[-+*]|[0-9]{1,9}[.)])[ \t]+").unwrap()
    });
    let line = raw.lines().next()?;
    let prefix = PREFIX.find(line)?.end();
    let (marker, checked) = task_box_marker(&line[prefix..])?;
    Some((prefix + marker.start..prefix + marker.end, checked))
}

pub fn set_task(source: &str, marker: Range<usize>, checked: bool) -> Option<String> {
    if marker.start == 0 || marker.end >= source.len() {
        return None;
    }
    let (local, _) = task_box_marker(source.get(marker.start - 1..)?)?;
    if marker.start - 1 + local.end != marker.end {
        return None;
    }
    let mut result = source.to_string();
    result.replace_range(marker, if checked { "x" } else { " " });
    Some(result)
}
