//! Reading presentation with source locations retained across links and transclusions.
use crate::index::{self, Index, Resolution};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

fn has_uri_prefix(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
    })
}

pub fn is_external_link(value: &str) -> bool {
    has_uri_prefix(value, &["http://", "https://", "mailto:"])
}

pub fn is_remote_image(value: &str) -> bool {
    has_uri_prefix(value, &["http://", "https://", "data:"])
}

#[derive(Clone, Debug)]
pub struct Reference {
    pub from: PathBuf,
    pub target: String,
    pub wiki: bool,
}
#[derive(Clone, Debug)]
pub struct TaskTarget {
    pub rendered_start: usize,
    pub path: PathBuf,
    pub marker: Range<usize>,
    pub baseline: Arc<str>,
}
#[derive(Clone, Debug, Default)]
pub struct ReadingDocument {
    pub markdown: String,
    pub references: Vec<Reference>,
    pub tasks: Vec<TaskTarget>,
    locations: Vec<SourceMap>,
    sources: BTreeMap<PathBuf, Arc<str>>,
}
#[derive(Clone, Debug)]
struct SourceMap {
    output: Range<usize>,
    path: PathBuf,
    start: usize,
    source_end: usize,
}
impl ReadingDocument {
    /// A syntax-validated literal graphic needs no link/footnote expansion.
    /// Retain its source mapping without scanning the whole note again.
    pub(crate) fn graphic_fragment(path: &Path, source: Arc<str>, range: Range<usize>) -> Self {
        let markdown = source[range.clone()].to_owned();
        Self {
            locations: vec![SourceMap {
                output: 0..markdown.len(),
                path: path.to_path_buf(),
                start: range.start,
                source_end: range.end,
            }],
            markdown,
            sources: BTreeMap::from([(path.to_path_buf(), source)]),
            ..Default::default()
        }
    }

    pub fn footnote_definition_targets(&self) -> Vec<(usize, PathBuf, usize)> {
        index::parse(&self.markdown)
            .footnote_definitions
            .into_iter()
            .filter_map(|(range, _)| {
                let origin = self.origin(range.start).or_else(|| {
                    self.locations
                        .iter()
                        .find(|m| range.start <= m.output.start && m.output.start < range.end)
                        .map(|m| (m.path.clone(), m.start))
                })?;
                Some((range.start, origin.0, origin.1))
            })
            .collect()
    }
    fn origin(&self, offset: usize) -> Option<(PathBuf, usize)> {
        let map = self.locations.iter().find(|m| m.output.contains(&offset))?;
        Some((
            map.path.clone(),
            if map.source_end - map.start == map.output.len() {
                map.start + offset - map.output.start
            } else {
                map.start
            },
        ))
    }

    /// Reference-order numbering, including reachable references in definitions.
    pub fn footnote_numbers(&self) -> BTreeMap<(PathBuf, usize), usize> {
        use markdown_parser::mdast::Node;
        fn definitions<'a>(node: &'a Node, out: &mut BTreeMap<String, &'a Node>) {
            if let Node::FootnoteDefinition(n) = node {
                out.entry(n.identifier.clone()).or_insert(node);
            }
            if let Some(children) = node.children() {
                for n in children {
                    definitions(n, out);
                }
            }
        }
        fn references(node: &Node, skip_definitions: bool, out: &mut Vec<(String, usize)>) {
            if skip_definitions && matches!(node, Node::FootnoteDefinition(_)) {
                return;
            }
            if let Node::FootnoteReference(n) = node
                && let Some(p) = &n.position
            {
                out.push((n.identifier.clone(), p.start.offset));
            }
            if let Some(children) = node.children() {
                for n in children {
                    references(n, skip_definitions, out);
                }
            }
        }
        let snapshot = crate::syntax::Snapshot::new(&self.markdown);
        let Some(ast) = snapshot.ast.as_deref() else {
            return BTreeMap::new();
        };
        let mut defs = BTreeMap::new();
        definitions(ast, &mut defs);
        let mut ordered = vec![];
        references(ast, true, &mut ordered);
        let mut ids = BTreeMap::new();
        let mut queue = vec![];
        let mut cursor = 0;
        loop {
            for (id, _) in ordered.drain(..) {
                if defs.contains_key(&id) && !ids.contains_key(&id) {
                    queue.push(id.clone());
                    ids.insert(id, queue.len());
                }
            }
            if cursor >= queue.len() {
                break;
            }
            if let Some(children) = defs[&queue[cursor]].children() {
                for n in children {
                    references(n, true, &mut ordered);
                }
            }
            cursor += 1;
        }
        let mut all = vec![];
        references(ast, false, &mut all);
        all.into_iter()
            .filter_map(|(id, offset)| Some((self.origin(offset)?, *ids.get(&id)?)))
            .collect()
    }

    pub fn footnote_overrides(
        &self,
        numbers: &BTreeMap<(PathBuf, usize), usize>,
    ) -> BTreeMap<String, usize> {
        fn collect(node: &markdown_parser::mdast::Node, out: &mut Vec<(String, usize)>) {
            if let markdown_parser::mdast::Node::FootnoteReference(n) = node
                && let Some(p) = &n.position
            {
                out.push((n.identifier.clone(), p.start.offset));
            }
            if let Some(children) = node.children() {
                for n in children {
                    collect(n, out);
                }
            }
        }
        let snapshot = crate::syntax::Snapshot::new(&self.markdown);
        let mut refs = vec![];
        if let Some(ast) = snapshot.ast.as_deref() {
            collect(ast, &mut refs);
        }
        refs.into_iter()
            .filter_map(|(id, offset)| Some((id, *numbers.get(&self.origin(offset)?)?)))
            .collect()
    }
    pub fn source_matches(&self, path: &Path, text: &str) -> bool {
        self.sources.get(path).is_some_and(|s| s.as_ref() == text)
    }
    pub fn output_offset(&self, path: &Path, source: usize) -> Option<usize> {
        if let Some(m) = self
            .locations
            .iter()
            .find(|m| m.path == path && m.start <= source && source <= m.source_end)
        {
            return Some(if m.source_end - m.start == m.output.len() {
                m.output.start + source - m.start
            } else {
                m.output.start
            });
        }
        self.locations
            .iter()
            .filter(|m| m.path == path)
            .min_by_key(|m| m.start.abs_diff(source))
            .map(|m| m.output.start)
    }
}
struct Builder<'a> {
    index: &'a Index,
    output: ReadingDocument,
    maps: Vec<SourceMap>,
    sources: BTreeMap<PathBuf, Arc<str>>,
    stack: Vec<(PathBuf, Range<usize>)>,
    quote: usize,
    line_start: bool,
    embedded_bytes: usize,
    footnote_namespaces: BTreeMap<PathBuf, usize>,
    footnote_identifiers: BTreeMap<(PathBuf, String), usize>,
    included_footnotes: std::collections::BTreeSet<(PathBuf, String)>,
    footnote_indent: usize,
    snapshots: BTreeMap<PathBuf, Arc<crate::syntax::Snapshot>>,
}
enum Action {
    Wiki(index::WikiLink, bool),
    Destination(String),
    Remove,
    Literal(String),
    InlineFootnote(crate::syntax::InlineFootnote, String),
    ReferenceImage(String, String, Option<String>),
    ReferenceLink(String, Option<String>),
}

fn reference_actions(ast: &markdown_parser::mdast::Node) -> Vec<(Range<usize>, Action)> {
    use markdown_parser::mdast::Node;
    fn definitions(node: &Node, out: &mut BTreeMap<String, (String, Option<String>)>) {
        if let Node::Definition(n) = node {
            out.entry(n.identifier.to_lowercase())
                .or_insert_with(|| (n.url.clone(), n.title.clone()));
        }
        if let Some(children) = node.children() {
            for child in children {
                definitions(child, out);
            }
        }
    }
    fn images(
        node: &Node,
        definitions: &BTreeMap<String, (String, Option<String>)>,
        out: &mut Vec<(Range<usize>, Action)>,
    ) {
        if let Node::ImageReference(n) = node
            && let Some(p) = &n.position
            && let Some((url, title)) = definitions.get(&n.identifier.to_lowercase())
        {
            out.push((
                p.start.offset..p.end.offset,
                Action::ReferenceImage(n.alt.clone(), url.clone(), title.clone()),
            ));
        }
        if let Node::LinkReference(n) = node
            && let Some(p) = &n.position
            && let Some(last) = n.children.last().and_then(Node::position)
            && let Some((url, title)) = definitions.get(&n.identifier.to_lowercase())
        {
            // Keep formatted label text available to other inline actions.
            out.push((
                last.end.offset..p.end.offset,
                Action::ReferenceLink(url.clone(), title.clone()),
            ));
        }
        if let Some(children) = node.children() {
            for child in children {
                images(child, definitions, out);
            }
        }
    }
    let mut defs = BTreeMap::new();
    definitions(ast, &mut defs);
    let mut actions = vec![];
    images(ast, &defs, &mut actions);
    actions
}
fn reference_title(title: Option<String>) -> String {
    title
        .map(|title| format!(" \"{}\"", title.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_default()
}

fn label(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}
fn image_target(target: &str) -> bool {
    Path::new(target.split('#').next().unwrap_or(""))
        .extension()
        .is_some_and(|e| {
            matches!(
                e.to_string_lossy().to_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
            )
        })
}
pub fn attachment_target(target: &str) -> bool {
    image_target(target)
        || Path::new(target.split('#').next().unwrap_or(""))
            .extension()
            .is_some_and(|e| {
                matches!(
                    e.to_string_lossy().to_lowercase().as_str(),
                    "pdf" | "mp3" | "wav" | "ogg" | "m4a" | "mp4" | "webm" | "mov" | "flac"
                )
            })
}
impl Builder<'_> {
    fn push(&mut self, text: &str, origin: Option<(&Path, usize)>) {
        let mut offset = 0;
        for part in text.split_inclusive('\n') {
            if self.line_start && self.quote > 0 {
                self.output.markdown.push_str(&"> ".repeat(self.quote));
            }
            if self.line_start && self.footnote_indent > 0 {
                self.output
                    .markdown
                    .push_str(&"    ".repeat(self.footnote_indent));
            }
            let start = self.output.markdown.len();
            self.output.markdown.push_str(part);
            if let Some((path, source)) = origin {
                self.maps.push(SourceMap {
                    output: start..self.output.markdown.len(),
                    path: path.to_path_buf(),
                    start: source + offset,
                    source_end: source + offset + part.len(),
                });
            }
            self.line_start = part.ends_with('\n');
            offset += part.len();
        }
    }
    fn reference(&mut self, from: &Path, target: String, wiki: bool) -> String {
        let id = self.output.references.len();
        self.output.references.push(Reference {
            from: from.to_path_buf(),
            target,
            wiki,
        });
        format!("inkstone-reference:{id}")
    }
    fn note(&mut self, path: &Path, source: Arc<str>, range: Range<usize>) {
        if self.stack.len() >= 8 || self.stack.iter().any(|(p, r)| p == path && *r == range) {
            self.push("*检测到循环嵌入，已停止展开。*\n", None);
            return;
        }
        if !self.stack.is_empty() {
            if self.embedded_bytes.saturating_add(range.len()) > 2_000_000 {
                self.push("*嵌入内容超过显示上限，请点击标题打开原笔记。*\n", None);
                return;
            }
            self.embedded_bytes += range.len();
        }
        self.stack.push((path.to_path_buf(), range.clone()));
        self.sources.insert(path.to_path_buf(), source.clone());
        let snapshot = if let Some(cached) = self.snapshots.get(path).filter(|s| s.source == source)
        {
            cached.clone()
        } else {
            let snapshot = Arc::new(crate::syntax::Snapshot::new(&source));
            self.snapshots.insert(path.to_path_buf(), snapshot.clone());
            snapshot
        };
        let parsed = index::parse_snapshot(&snapshot);
        let html_destinations = snapshot
            .ast
            .as_deref()
            .map(|ast| crate::syntax::html_destinations(ast, &snapshot.structural))
            .unwrap_or_default();
        let mut actions: Vec<_> = parsed
            .comments
            .iter()
            .map(|comment| {
                (
                    comment.range.clone(),
                    if comment.block {
                        Action::Remove
                    } else {
                        Action::Literal("<!---->".into())
                    },
                )
            })
            .collect();
        if let Some(ast) = snapshot.ast.as_deref() {
            actions.extend(reference_actions(ast));
        }
        let mut definitions = BTreeMap::new();
        for (span, id) in &parsed.footnote_definitions {
            if definitions.contains_key(id) {
                actions.push((span.clone(), Action::Remove));
            } else {
                definitions.insert(id.clone(), span.clone());
            }
        }
        for (id, definition) in &definitions {
            if range.start <= definition.start && definition.end <= range.end {
                self.included_footnotes
                    .insert((path.to_path_buf(), id.clone()));
            }
        }
        let mut needed: Vec<_> = parsed
            .footnote_references
            .iter()
            .filter(|(r, _)| range.start <= r.start && r.end <= range.end)
            .filter(|(r, _)| {
                !parsed
                    .footnote_definitions
                    .iter()
                    .any(|(d, _)| d != &range && d.start <= r.start && r.end <= d.end)
            })
            .map(|(_, id)| id.clone())
            .collect();
        let mut seen: std::collections::BTreeSet<_> = needed.iter().cloned().collect();
        let mut i = 0;
        while i < needed.len() {
            if let Some(definition) = definitions.get(&needed[i]) {
                for (r, id) in &parsed.footnote_references {
                    if definition.start <= r.start
                        && r.end <= definition.end
                        && seen.insert(id.clone())
                    {
                        needed.push(id.clone());
                    }
                }
            }
            i += 1;
        }
        let next = self.footnote_namespaces.len();
        let namespace = *self
            .footnote_namespaces
            .entry(path.to_path_buf())
            .or_insert(next);
        for note in &parsed.inline_footnotes {
            actions.push((
                note.range.clone(),
                Action::InlineFootnote(
                    note.clone(),
                    format!("inline-fn{namespace}-{}", note.range.start),
                ),
            ));
        }
        let mut inline_definitions = vec![];
        for (span, identifier) in &parsed.footnotes {
            if definitions.contains_key(identifier) {
                let next = self.footnote_identifiers.len();
                let id = *self
                    .footnote_identifiers
                    .entry((path.to_path_buf(), identifier.clone()))
                    .or_insert(next);
                actions.push((span.clone(), Action::Literal(format!("fn{namespace}-{id}"))));
            }
        }
        for link in parsed.links {
            let bang = link
                .range
                .start
                .checked_sub(1)
                .filter(|i| source.as_bytes()[*i] == b'!');
            let embed = bang.is_some_and(|i| {
                source[..i]
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\\')
                    .count()
                    % 2
                    == 0
            });
            let mut range = link.range.clone();
            if embed {
                range.start -= 1;
            }
            actions.push((range, Action::Wiki(link, embed)));
        }
        for (range, url) in parsed.destinations {
            actions.push((range, Action::Destination(url)));
        }
        actions.extend(
            html_destinations
                .into_iter()
                .map(|(range, url)| (range, Action::Destination(url))),
        );
        for span in crate::markdown::spans_snapshot(&snapshot)
            .into_iter()
            .filter(|s| s.kind == crate::markdown::Kind::Highlight)
        {
            actions.push((span.markers[0].clone(), Action::Literal("<mark>".into())));
            actions.push((span.markers[1].clone(), Action::Literal("</mark>".into())));
        }
        for block in parsed.blocks {
            actions.push((block.marker, Action::Remove));
        }
        actions.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
        let mut cursor = range
            .start
            .max(crate::properties::block(&source).map_or(0, |r| r.end));
        for (span, action) in actions {
            if span.start < cursor || span.end > range.end {
                continue;
            }
            self.push(&source[cursor..span.start], Some((path, cursor)));
            match action {
                Action::Destination(url) => {
                    let uri = self.reference(path, url, false);
                    self.push(&uri, None);
                }
                Action::Remove => (),
                Action::Literal(text) => self.push(&text, None),
                Action::ReferenceLink(url, title) => {
                    let uri = self.reference(path, url, false);
                    self.push(&format!("]({uri}{})", reference_title(title)), None);
                }
                Action::ReferenceImage(alt, url, title) => {
                    let uri = self.reference(path, url, false);
                    let title = reference_title(title);
                    let start = self.output.markdown.len();
                    self.push(&format!("![{}]({uri}{title})", label(&alt)), None);
                    self.maps.push(SourceMap {
                        output: start..self.output.markdown.len(),
                        path: path.to_path_buf(),
                        start: span.start,
                        source_end: span.end,
                    });
                }
                Action::InlineFootnote(note, id) => {
                    let start = self.output.markdown.len();
                    self.push(&format!("[^{id}]"), None);
                    // Atomic generated reference: map its whole visible marker
                    // back to the original inline reference rather than inventing offsets.
                    self.maps.push(SourceMap {
                        output: start..self.output.markdown.len(),
                        path: path.to_path_buf(),
                        start: note.range.start,
                        source_end: note.range.end,
                    });
                    inline_definitions.push((note, id));
                }
                Action::Wiki(link, embed) => {
                    let uri = self.reference(path, link.target.clone(), true);
                    if embed && image_target(&link.target) {
                        let dimensions = link
                            .label
                            .split_once('x')
                            .map(|(w, h)| (w.parse::<u32>().ok(), h.parse::<u32>().ok()))
                            .unwrap_or_else(|| (link.label.parse::<u32>().ok(), None));
                        if let (Some(width), height) = dimensions
                            && (1..=10000).contains(&width)
                        {
                            let height = height
                                .filter(|h| (1..=10000).contains(h))
                                .map(|h| format!(" height=\"{h}\""))
                                .unwrap_or_default();
                            self.push(
                                &format!("<img src=\"{uri}\" width=\"{width}\"{height} />"),
                                None,
                            );
                        } else {
                            self.push(&format!("![{}]({uri})", label(&link.label)), None);
                        }
                    } else if embed && attachment_target(&link.target) {
                        self.push(
                            &format!("\n\n[附件：{}]({uri})\n\n", label(&link.label)),
                            None,
                        );
                    } else if embed {
                        let target = match self.index.resolve(path, &link.target) {
                            Resolution::Found(p) => Some(p),
                            Resolution::Missing(p) if self.sources.contains_key(&p) => Some(p),
                            _ => None,
                        };
                        self.push("\n\n", None);
                        self.quote += 1;
                        self.push(&format!("[{}]({uri})\n\n", label(&link.label)), None);
                        if let Some(target) = target {
                            if let Some(text) = self.sources.get(&target).cloned().or_else(|| {
                                self.index
                                    .notes
                                    .get(&target)
                                    .map(|n| Arc::<str>::from(n.text.as_str()))
                            }) {
                                let section = link.target.split_once('#').map(|(_, s)| s);
                                let part = section.map_or_else(
                                    || Some(0..text.len()),
                                    |fragment| {
                                        index::anchor_range(&text, &index::parse(&text), fragment)
                                    },
                                );
                                if let Some(part) = part {
                                    self.note(&target, text, part);
                                } else {
                                    self.push("*未找到指定的标题或块。*\n", None);
                                }
                            } else {
                                self.push("*未找到嵌入的笔记。*\n", None);
                            }
                        } else {
                            self.push("*未找到唯一的嵌入目标。*\n", None);
                        }
                        self.push("\n", None);
                        self.quote -= 1;
                        self.push("\n", None);
                    } else {
                        self.push(&format!("[{}]({uri})", label(&link.label)), None);
                    }
                }
            }
            cursor = span.end;
        }
        if cursor <= range.end {
            self.push(&source[cursor..range.end], Some((path, cursor)));
        }
        for (note, id) in inline_definitions {
            if !self
                .included_footnotes
                .insert((path.to_path_buf(), id.clone()))
            {
                continue;
            }
            self.push(&format!("\n\n[^{id}]: "), None);
            self.footnote_indent += 1;
            self.note(path, source.clone(), note.content);
            self.footnote_indent -= 1;
            self.push("\n", None);
        }
        for id in needed {
            if !self
                .included_footnotes
                .contains(&(path.to_path_buf(), id.clone()))
                && let Some(definition) = definitions.get(&id)
            {
                self.included_footnotes.insert((path.to_path_buf(), id));
                self.push("\n\n", None);
                self.note(path, source.clone(), definition.clone());
            }
        }
        self.stack.pop();
    }
    fn finish(mut self) -> ReadingDocument {
        let parsed = index::parse(&self.output.markdown);
        for task in parsed.tasks {
            if let Some(map) = self
                .maps
                .get(
                    self.maps
                        .partition_point(|m| m.output.end <= task.marker.start),
                )
                .filter(|m| m.output.start <= task.marker.start && task.marker.end <= m.output.end)
                && let Some(baseline) = self.sources.get(&map.path)
            {
                let start = map.start + task.marker.start - map.output.start;
                self.output.tasks.push(TaskTarget {
                    rendered_start: task.start,
                    path: map.path.clone(),
                    marker: start..start + task.marker.len(),
                    baseline: baseline.clone(),
                });
            }
        }
        self.output.locations = self.maps;
        self.output.sources = self.sources;
        self.output
    }
}
pub fn reading_document(index: &Index, path: &Path, source: &str) -> ReadingDocument {
    reading_snapshot(
        index,
        path,
        Arc::new(crate::syntax::Snapshot::new(source)),
        0..source.len(),
    )
}

pub fn reading_snapshot(
    index: &Index,
    path: &Path,
    snapshot: Arc<crate::syntax::Snapshot>,
    range: Range<usize>,
) -> ReadingDocument {
    let source = snapshot.source.clone();
    let mut builder = Builder {
        index,
        output: ReadingDocument::default(),
        maps: vec![],
        sources: BTreeMap::new(),
        stack: vec![],
        quote: 0,
        line_start: true,
        embedded_bytes: 0,
        footnote_namespaces: BTreeMap::new(),
        footnote_identifiers: BTreeMap::new(),
        included_footnotes: Default::default(),
        footnote_indent: 0,
        snapshots: BTreeMap::from([(path.to_path_buf(), snapshot)]),
    };
    builder.note(path, source, range);
    builder.finish()
}

/// Resolve local media within the vault, respecting Markdown-relative and wiki-root paths.
pub fn asset_path(root: &Path, reference: &Reference, files: &[PathBuf]) -> Option<PathBuf> {
    let target = reference.target.split('#').next()?;
    let decoded = if reference.wiki {
        target.into()
    } else {
        percent_encoding::percent_decode_str(target)
            .decode_utf8()
            .ok()?
    };
    if decoded.contains(':') {
        return None;
    }
    let relative = Path::new(decoded.as_ref());
    let mut candidates = vec![];
    if !reference.wiki
        || !decoded.contains('/')
        || decoded.starts_with("./")
        || decoded.starts_with("../")
    {
        candidates.push(
            reference
                .from
                .parent()
                .unwrap_or(Path::new(""))
                .join(relative),
        );
    }
    if reference.wiki || decoded.starts_with('/') {
        candidates.push(PathBuf::from(decoded.trim_start_matches('/')));
    }
    if !decoded.contains(['/', '\\']) {
        let matches: Vec<_> = files
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&decoded))
            })
            .collect();
        if matches.len() == 1 {
            candidates.push(matches[0].clone());
        }
    }
    for candidate in candidates {
        let mut clean = PathBuf::new();
        let mut valid = true;
        for c in candidate.components() {
            match c {
                Component::Normal(s) => clean.push(s),
                Component::CurDir => (),
                Component::ParentDir => {
                    if !clean.pop() {
                        valid = false;
                        break;
                    }
                }
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if valid
            && let Ok(path) = std::fs::canonicalize(root.join(clean))
            && path.starts_with(root)
            && path.is_file()
        {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_links_keep_formatted_labels_when_definitions_are_outside_the_section() {
        let source = "# 显示\r\n\r\n[**粗体** ==高亮==][DEST]、[简写][]、[快捷]\r\n\r\n# 定义\r\n\r\n[dest]: ../target.md#标题 \"链接标题\"\r\n[简写]: short.md\r\n[快捷]: quick.md\r\n";
        let index = crate::index::Index::default();
        let path = std::path::Path::new("folder/note.md");
        let section = 0..source.find("# 定义").unwrap();
        let document = super::reading_snapshot(
            &index,
            path,
            std::sync::Arc::new(crate::syntax::Snapshot::new(source)),
            section,
        );
        assert!(
            document
                .markdown
                .contains("[**粗体** <mark>高亮</mark>](inkstone-reference:"),
            "{}",
            document.markdown
        );
        assert!(document.markdown.contains("\"链接标题\""));
        let parsed = crate::index::parse(&document.markdown);
        assert_eq!(parsed.standard_links.len(), 3);
        assert_eq!(
            document
                .references
                .iter()
                .map(|r| r.target.as_str())
                .collect::<Vec<_>>(),
            ["../target.md#标题", "short.md", "quick.md"]
        );
        assert!(document.references.iter().all(|r| r.from == path));
        assert_eq!(
            document.output_offset(path, source.find("粗体").unwrap()),
            document.markdown.find("粗体")
        );
        assert!(document.source_matches(path, source));
    }
    use super::*;
    #[test]
    fn html_links_and_images_keep_source_context_and_ignore_quoted_fake_attributes() {
        let source = r#"<div title="href='fake.md'"><a title="> title" href="真实.md?x=1&amp;y=2">跳转</a><img src='图 片.png'></div>
<script>const x = '<a href="bad.md">';</script>
<a href="after.md">after</a>"#;
        let document = reading_document(&Index::default(), Path::new("子目录/note.md"), source);
        assert_eq!(document.references.len(), 3, "{:?}", document.references);
        assert_eq!(document.references[0].target, "真实.md?x=1&y=2");
        assert_eq!(document.references[1].target, "图 片.png");
        assert_eq!(document.references[2].target, "after.md");
        assert!(
            document
                .references
                .iter()
                .all(|r| r.from == Path::new("子目录/note.md"))
        );
        assert!(document.markdown.contains("href=\"inkstone-reference:0\""));
        assert!(document.markdown.contains("src='inkstone-reference:1'"));
        assert!(document.source_matches(Path::new("子目录/note.md"), source));
    }
    #[test]
    fn inline_footnotes_convert_only_for_display_and_keep_body_links_and_source_mapping() {
        let source = "A^[短] B[^long] C^[**文字** [链接](target.md)]\r\n\r\n[^long]: named\r\n";
        let path = Path::new("note.md");
        let document = reading_document(&Index::default(), path, source);
        let parsed = index::parse(&document.markdown);
        assert_eq!(
            parsed.footnote_definitions.len(),
            3,
            "{}",
            document.markdown
        );
        assert_eq!(parsed.footnote_references.len(), 3);
        assert_eq!(document.references[0].target, "target.md");
        assert!(document.markdown.contains("**文字**"));
        assert!(document.source_matches(path, source));
        let start = source.find("^[短]").unwrap();
        assert_eq!(document.output_offset(path, start), Some(1));
        assert_eq!(document.output_offset(path, start + 2), Some(1));
    }
    #[test]
    fn uri_schemes_ignore_case_without_reclassifying_local_names() {
        for value in [
            "HTTPS://example.com/CaseSensitive?q=AbC",
            "HtTp://example.com",
            "MAILTO:User@example.com",
        ] {
            assert!(is_external_link(value));
        }
        for value in [
            "https-note.md",
            "httpx://example.com",
            "文件.md",
            "目录/HTTP:notes",
            "data:text/plain,hello",
            "javascript:alert(1)",
        ] {
            assert!(!is_external_link(value));
        }
        assert!(is_remote_image("DaTa:image/png;base64,AAAA"));
        assert!(is_remote_image("HTTPS://example.com/Picture.PNG"));
        assert!(!is_remote_image("MAILTO:User@example.com"));
        assert!(!is_remote_image("附件/图片.png"));
    }
    #[test]
    fn imported_attachment_formats_resolve_with_catalog_and_collisions() {
        use crate::{
            locations::{LinkFormat, attachment_link},
            vault::Vault,
        };
        let root = std::env::temp_dir().join(format!(
            "inkstone-asset-links-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let files = [
            "a/same.png",
            "same.png",
            "media/unique.png",
            "a/sub/中文 #1.png",
            "one/duplicate.png",
            "two/duplicate.png",
        ];
        for path in files {
            let absolute = root.join(path);
            std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
            std::fs::write(absolute, b"fixture").unwrap();
        }
        let recovery = root.with_extension("recovery");
        let vault = Vault::open(&root, &recovery).unwrap();
        let index = Index::build(&vault).unwrap();
        assert!(index.notes.is_empty());
        assert_eq!(index.files.len(), files.len());
        let from = Path::new("a/note.md");
        for format in [
            LinkFormat::Shortest,
            LinkFormat::Relative,
            LinkFormat::Absolute,
        ] {
            for markdown in [false, true] {
                for path in files {
                    let link = attachment_link(
                        from,
                        Path::new(path),
                        &index.files,
                        format,
                        markdown,
                        true,
                    );
                    let rendered = reading_document(&index, from, &link);
                    assert_eq!(rendered.references.len(), 1, "{link}");
                    assert_eq!(
                        asset_path(&vault.root, &rendered.references[0], &index.files),
                        Some(std::fs::canonicalize(root.join(path)).unwrap()),
                        "{link}"
                    );
                }
            }
        }
        let reference = Reference {
            from: "a/note.md".into(),
            target: "duplicate.png".into(),
            wiki: true,
        };
        assert!(asset_path(&vault.root, &reference, &index.files).is_none());
        let outside = Reference {
            from: from.into(),
            target: "../../outside.png".into(),
            wiki: false,
        };
        assert!(asset_path(&vault.root, &outside, &index.files).is_none());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(recovery).unwrap();
    }
    #[test]
    fn embedded_sections_keep_distinct_footnotes_and_pull_their_definitions() {
        let mut index = Index::default();
        let child = "# Section\nchild[^same]\n\n# Next\nother\n\n[^same]: CHILD";
        index.update("child.md".into(), child.into());
        let source = "root[^same]\n\n![[child#Section]]\n\n[^same]: ROOT";
        let rendered = reading_document(&index, Path::new("root.md"), source);
        assert!(rendered.markdown.contains("CHILD"));
        assert!(rendered.markdown.contains("ROOT"));
        let parsed = index::parse(&rendered.markdown);
        let ids: std::collections::BTreeSet<_> = parsed
            .footnote_definitions
            .iter()
            .map(|(_, id)| id)
            .collect();
        assert_eq!(ids.len(), 2, "{}", rendered.markdown);
    }
    #[test]
    fn embedded_sections_tasks_and_links_retain_original_sources() {
        let mut index = Index::default();
        let target = "# 第一节\n- [ ] 中文😀 [[其他]]\n\n# 第二节\n不应嵌入\n";
        index.update("目录/目标.md".into(), target.into());
        let source =
            "---\ntitle: 页面\n---\n[[目标|短名]]\n- [x] 本地任务\n![[目录/目标#第一节]]\n";
        let rendered = reading_document(&index, Path::new("首页.md"), source);
        assert!(rendered.markdown.contains("中文😀"));
        assert!(!rendered.markdown.contains("不应嵌入"));
        assert!(!rendered.markdown.contains("title:"));
        assert_eq!(rendered.tasks.len(), 2);
        assert_eq!(rendered.tasks[0].path, PathBuf::from("首页.md"));
        assert_eq!(&source[rendered.tasks[0].marker.clone()], "x");
        assert_eq!(rendered.tasks[1].path, PathBuf::from("目录/目标.md"));
        assert_eq!(&target[rendered.tasks[1].marker.clone()], " ");
        assert!(
            rendered
                .references
                .iter()
                .any(|r| r.from == Path::new("目录/目标.md") && r.target == "其他")
        );
    }
    #[test]
    fn cyclic_embeds_stop_and_code_is_literal() {
        let mut index = Index::default();
        index.update("a.md".into(), "![[b]]".into());
        index.update("b.md".into(), "![[a]]".into());
        let source = "![[b]]\n`![[b]]`\n";
        let r = reading_document(&index, Path::new("a.md"), source);
        assert!(r.markdown.contains("循环嵌入"));
        assert!(r.markdown.contains("`![[b]]`"));
        assert!(r.markdown.len() < 2000);
    }
}
