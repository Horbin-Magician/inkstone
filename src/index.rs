use crate::vault::{Vault, VaultError};
use markdown_parser::mdast::Node;
const LINK_PATH_ESCAPE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'#')
    .add(b'%')
    .add(b'(')
    .add(b')');
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikiLink {
    pub range: Range<usize>,
    pub target: String,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub offset: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskItem {
    pub start: usize,
    pub marker: Range<usize>,
    pub checked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockReference {
    pub id: String,
    pub range: Range<usize>,
    pub marker: Range<usize>,
}
#[derive(Clone, Debug, Default)]
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
    pub aliases: Vec<String>,
    pub standard_links: Vec<(String, usize)>,
    pub destinations: Vec<(Range<usize>, String)>,
    pub tasks: Vec<TaskItem>,
    pub blocks: Vec<BlockReference>,
    /// Zero-based header row through the exclusive body end row.
    pub folds: Vec<Range<usize>>,
    pub fold_regions: Vec<(Range<usize>, FoldKind)>,
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Debug)]
pub struct IndexedNote {
    pub text: String,
    pub parsed: ParsedNote,
    pub times: crate::file_order::FileTimes,
}
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub notes: BTreeMap<PathBuf, std::sync::Arc<IndexedNote>>,
    pub files: Vec<PathBuf>,
    /// Files excluded from searchable content, with actionable read diagnostics.
    pub errors: BTreeMap<PathBuf, String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Found(PathBuf),
    Missing(PathBuf),
    Ambiguous(Vec<PathBuf>),
    Invalid,
}
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub path: PathBuf,
    pub display_name: Option<String>,
    pub offset: usize,
    pub line: usize,
    pub excerpt: String,
    pub highlights: Vec<Range<usize>>,
    pub title_highlights: Vec<Range<usize>>,
}
fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}
fn normalized(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => result.push(p),
            Component::CurDir => (),
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(result)
}
impl Index {
    pub fn anchor_range(&self, path: &Path, fragment: &str) -> Option<Range<usize>> {
        let note = self.notes.get(path)?;
        anchor_range(&note.text, &note.parsed, fragment)
    }
    /// Plan all references against the pre-move index, including outgoing relative links.
    pub fn relocation_edits(
        &self,
        old: &Path,
        new: &Path,
        folder: bool,
        asset_root: Option<&Path>,
    ) -> Vec<(PathBuf, String, String)> {
        let moved = |path: &Path| {
            if folder {
                path.strip_prefix(old)
                    .map_or_else(|_| path.to_path_buf(), |suffix| new.join(suffix))
            } else if path == old {
                new.to_path_buf()
            } else {
                path.to_path_buf()
            }
        };
        let asset = |from: &Path, target: &str, wiki| {
            let root = asset_root?;
            crate::rendering::asset_path(
                root,
                &crate::rendering::Reference {
                    from: from.to_path_buf(),
                    target: target.into(),
                    wiki,
                },
                &self.files,
            )?
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
        };
        let mut edits = vec![];
        for (from, note) in &self.notes {
            let source = moved(from);
            let mut replacements: Vec<(Range<usize>, String)> = vec![];
            for link in &note.parsed.links {
                if link.target.starts_with('#') {
                    continue;
                }
                let target = match self.resolve(from, &link.target) {
                    Resolution::Found(path) => path,
                    _ => match asset(from, &link.target, true) {
                        Some(path) => path,
                        None => continue,
                    },
                };
                let destination = moved(&target);
                if destination == target && source == *from {
                    continue;
                }
                let fragment = link
                    .target
                    .split_once('#')
                    .map_or(String::new(), |(_, s)| format!("#{s}"));
                let raw = &note.text[link.range.clone()];
                let alias = raw[2..raw.len() - 2]
                    .split_once('|')
                    .map_or(String::new(), |(_, s)| format!("|{s}"));
                let destination = if destination
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                {
                    destination.with_extension("")
                } else {
                    destination
                };
                let target = destination.to_string_lossy().replace('\\', "/");
                replacements.push((
                    link.range.clone(),
                    format!("[[/{target}{fragment}{alias}]]"),
                ));
            }
            for (range, url) in &note.parsed.destinations {
                if url.starts_with('#') {
                    continue;
                }
                let target = match self.resolve_markdown(from, url).0 {
                    Resolution::Found(path) => path,
                    _ => match asset(from, url, false) {
                        Some(path) => path,
                        None => continue,
                    },
                };
                let destination = moved(&target);
                if destination == target && source == *from {
                    continue;
                }
                let parent = source.parent().unwrap_or(Path::new(""));
                let a: Vec<_> = parent.components().collect();
                let b: Vec<_> = destination.components().collect();
                let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
                let mut relative = PathBuf::new();
                for _ in common..a.len() {
                    relative.push("..");
                }
                for part in &b[common..] {
                    relative.push(part.as_os_str());
                }
                let path = relative.to_string_lossy().replace('\\', "/");
                let encoded =
                    percent_encoding::utf8_percent_encode(&path, LINK_PATH_ESCAPE).to_string();
                let fragment = url
                    .split_once('#')
                    .map_or(String::new(), |(_, s)| format!("#{s}"));
                replacements.push((range.clone(), format!("{encoded}{fragment}")));
            }
            if !replacements.is_empty() {
                replacements.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
                let mut after = note.text.clone();
                for (range, text) in replacements {
                    after.replace_range(range, &text);
                }
                if after != note.text {
                    edits.push((source, note.text.clone(), after));
                }
            }
        }
        edits
    }
    pub fn build(vault: &Vault) -> Result<Self, VaultError> {
        let mut index = Self {
            files: vault.scan_files()?,
            ..Default::default()
        };
        let paths: Vec<_> = index
            .files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .cloned()
            .collect();
        index.refresh_paths(vault, paths)?;
        Ok(index)
    }
    pub fn note_paths(&self) -> Vec<PathBuf> {
        self.notes
            .keys()
            .chain(self.errors.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn update(&mut self, path: PathBuf, text: String) {
        self.errors.remove(&path);
        let parsed = parse(&text);
        let times = self.notes.get(&path).map(|n| n.times).unwrap_or_default();
        self.notes.insert(
            path,
            std::sync::Arc::new(IndexedNote {
                text,
                parsed,
                times,
            }),
        );
    }
    fn refresh_file_times(&mut self, vault: &Vault, path: &Path) {
        let times = vault
            .path(path)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| crate::file_order::FileTimes {
                modified: m.modified().ok(),
                created: m.created().ok(),
            })
            .unwrap_or_default();
        if let Some(note) = self.notes.get_mut(path)
            && note.times != times
        {
            std::sync::Arc::make_mut(note).times = times;
        }
    }
    pub fn refresh_paths(
        &mut self,
        vault: &Vault,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<(), VaultError> {
        for path in paths {
            match vault.read(&path) {
                Ok(Some(text)) => {
                    self.errors.remove(&path);
                    if self.notes.get(&path).is_none_or(|n| n.text != text) {
                        self.update(path.clone(), text);
                    }
                    self.refresh_file_times(vault, &path);
                }
                Ok(None) => {
                    self.notes.remove(&path);
                    self.errors.remove(&path);
                }
                Err(error) => {
                    self.notes.remove(&path);
                    self.errors.insert(path, error.to_string());
                }
            }
        }
        Ok(())
    }
    pub fn resolve(&self, from: &Path, target: &str) -> Resolution {
        self.resolve_literal(from, target.split('#').next().unwrap_or(""))
    }
    /// Markdown destinations are relative to the containing document, unlike wiki paths.
    /// Decode the path and fragment separately so an encoded '#' stays in the filename.
    pub fn resolve_markdown(&self, from: &Path, href: &str) -> (Resolution, Option<String>) {
        let (path, fragment) = href
            .split_once('#')
            .map_or((href, None), |(p, f)| (p, Some(f)));
        let Ok(path) = percent_encoding::percent_decode_str(path).decode_utf8() else {
            return (Resolution::Invalid, None);
        };
        let heading = match fragment.map(|f| percent_encoding::percent_decode_str(f).decode_utf8())
        {
            Some(Ok(text)) => Some(text.into_owned()),
            Some(Err(_)) => return (Resolution::Invalid, None),
            None => None,
        };
        if path.is_empty() {
            return (Resolution::Found(from.into()), heading);
        }
        if Path::new(path.as_ref())
            .extension()
            .is_some_and(|e| !e.eq_ignore_ascii_case("md"))
        {
            return (Resolution::Invalid, heading);
        }
        let target = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("./{path}")
        };
        let resolved = self.resolve_literal(from, &target);
        // Obsidian's shortest Markdown links may name a unique note elsewhere in the vault.
        let resolved = if matches!(resolved, Resolution::Missing(_)) && !path.contains('/') {
            self.resolve_literal(from, &path)
        } else {
            resolved
        };
        (resolved, heading)
    }
    fn resolve_literal(&self, from: &Path, target: &str) -> Resolution {
        let target = target.trim().replace('\\', "/");
        if target.is_empty() {
            return Resolution::Found(from.into());
        }
        if target.contains(':') {
            return Resolution::Invalid;
        }
        let explicit = target.contains('/');
        let mut path = if target.starts_with("./") || target.starts_with("../") {
            from.parent().unwrap_or(Path::new("")).join(&target)
        } else if explicit {
            PathBuf::from(target.trim_start_matches('/'))
        } else {
            from.parent().unwrap_or(Path::new("")).join(&target)
        };
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("md"))
        {
            let mut name = path.into_os_string();
            name.push(".md");
            path = PathBuf::from(name);
        }
        let Some(path) = normalized(&path) else {
            return Resolution::Invalid;
        };
        if let Some(found) = self.notes.keys().find(|p| key(p) == key(&path)) {
            return Resolution::Found(found.clone());
        }
        if explicit {
            return Resolution::Missing(path);
        }
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let matches: Vec<_> = self
            .notes
            .keys()
            .filter(|p| {
                p.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
                    == stem
            })
            .cloned()
            .collect();
        match matches.len() {
            0 => {
                let aliases: Vec<_> = self
                    .notes
                    .iter()
                    .filter(|(_, n)| n.parsed.aliases.iter().any(|a| a.to_lowercase() == stem))
                    .map(|(p, _)| p.clone())
                    .collect();
                match aliases.len() {
                    0 => Resolution::Missing(path),
                    1 => Resolution::Found(aliases[0].clone()),
                    _ => Resolution::Ambiguous(aliases),
                }
            }
            1 => Resolution::Found(matches[0].clone()),
            _ => Resolution::Ambiguous(matches),
        }
    }
    pub fn filenames(&self, query: &str) -> Vec<SearchHit> {
        let query = query.trim().replace('\\', "/").to_lowercase();
        let mut matches = Vec::new();
        for (path, note) in &self.notes {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let mut best = if stem == query {
                Some((0, None))
            } else if stem.starts_with(&query) {
                Some((2, None))
            } else if key(path).contains(&query) {
                Some((4, None))
            } else {
                None
            };
            for alias in &note.parsed.aliases {
                let name = alias.to_lowercase();
                let rank = if name == query {
                    1
                } else if name.starts_with(&query) {
                    3
                } else if name.contains(&query) {
                    5
                } else {
                    continue;
                };
                if best.as_ref().is_none_or(|(old, _)| rank < *old) {
                    best = Some((rank, Some(alias.clone())));
                }
            }
            if let Some((rank, display_name)) = best {
                matches.push((
                    rank,
                    key(path),
                    SearchHit {
                        path: path.clone(),
                        display_name,
                        offset: 0,
                        line: 1,
                        excerpt: String::new(),
                        highlights: vec![],
                        title_highlights: vec![],
                    },
                ));
            }
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        matches
            .into_iter()
            .take(200)
            .map(|(_, _, hit)| hit)
            .collect()
    }
    #[cfg(test)]
    pub(crate) fn search(&self, query: &str) -> Vec<SearchHit> {
        self.try_search(query).unwrap_or_default()
    }
    #[cfg(test)]
    fn try_search(&self, query: &str) -> Result<Vec<SearchHit>, String> {
        self.search_with_case(query, false)
    }
    #[cfg(test)]
    fn search_with_case(
        &self,
        query: &str,
        case_sensitive: bool,
    ) -> Result<Vec<SearchHit>, String> {
        self.search_ordered(
            query,
            case_sensitive,
            crate::file_order::SortBy::Name,
            false,
        )
    }
    #[cfg(test)]
    fn search_ordered(
        &self,
        query: &str,
        case_sensitive: bool,
        by: crate::file_order::SortBy,
        descending: bool,
    ) -> Result<Vec<SearchHit>, String> {
        self.search_limited(query, case_sensitive, by, descending, 200)
    }
    pub fn search_limited(
        &self,
        query: &str,
        case_sensitive: bool,
        by: crate::file_order::SortBy,
        descending: bool,
        limit: usize,
    ) -> Result<Vec<SearchHit>, String> {
        if limit == 0 {
            return Ok(vec![]);
        }
        fn excerpt(text: &str, at: usize) -> String {
            let skip = text[..at].chars().count().saturating_sub(40);
            text.chars().skip(skip).take(120).collect()
        }
        let query = crate::search::Query::parse_with_case(query, case_sensitive)?;
        let mut hits = vec![];
        let mut notes: Vec<_> = self
            .notes
            .iter()
            .filter(|(path, note)| query.matches(path, &note.text, &note.parsed.tags))
            .collect();
        notes.sort_by(|(a, an), (b, bn)| {
            crate::file_order::compare(
                (&a.to_string_lossy(), false, an.times),
                (&b.to_string_lossy(), false, bn.times),
                by,
                descending,
            )
        });
        for (path, note) in notes {
            let title_highlights = query.title_highlights(path, &note.text, &note.parsed.tags);
            let before = hits.len();
            for found in
                query.matching_lines(path, &note.text, &note.parsed.tags, limit - hits.len())
            {
                hits.push(SearchHit {
                    path: path.clone(),
                    display_name: None,
                    offset: found.offset,
                    line: found.line,
                    excerpt: note.text[found.range].to_string(),
                    highlights: found.highlights,
                    title_highlights: title_highlights.clone(),
                });
            }
            if hits.len() == before {
                let at = query
                    .first_offset(path, &note.text, &note.parsed.tags)
                    .unwrap_or(0);
                let start = note.text[..at].rfind('\n').map_or(0, |i| i + 1);
                let end = note.text[at..]
                    .find('\n')
                    .map_or(note.text.len(), |i| at + i);
                hits.push(SearchHit {
                    path: path.clone(),
                    display_name: None,
                    offset: at,
                    line: note.text[..at]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1,
                    excerpt: excerpt(&note.text[start..end], at - start),
                    highlights: vec![],
                    title_highlights,
                });
            }
            if hits.len() == limit {
                return Ok(hits);
            }
        }
        Ok(hits)
    }
    pub fn backlinks(&self, target: &Path) -> Vec<PathBuf> {
        self.notes.iter().filter(|(from,note)| {
            note.parsed.links.iter().any(|l|matches!(self.resolve(from,&l.target),Resolution::Found(ref p)if key(p)==key(target))) ||
            note.parsed.standard_links.iter().any(|(url,_)|matches!(self.resolve_markdown(from,url).0,Resolution::Found(ref p)if key(p)==key(target)))
        }).map(|(p,_)|p.clone()).collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_snapshots_share_unchanged_notes_and_isolate_updates() {
        let mut old = Index::default();
        old.update("first.md".into(), "before".into());
        old.update("second.md".into(), "unchanged".into());
        let mut next = old.clone();
        assert!(std::sync::Arc::ptr_eq(
            &old.notes[Path::new("first.md")],
            &next.notes[Path::new("first.md")]
        ));
        next.update("first.md".into(), "after".into());
        assert_eq!(old.notes[Path::new("first.md")].text, "before");
        assert_eq!(next.notes[Path::new("first.md")].text, "after");
        assert!(std::sync::Arc::ptr_eq(
            &old.notes[Path::new("second.md")],
            &next.notes[Path::new("second.md")]
        ));
    }

    #[test]
    fn unreadable_notes_are_isolated_and_rejoin_after_repair() {
        let root = std::env::temp_dir().join(format!(
            "inkstone-index-errors-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("good.md"), "good").unwrap();
        std::fs::write(vault.root.join("bad.md"), [0xff]).unwrap();
        let mut index = Index::build(&vault).unwrap();
        assert_eq!(index.notes.len(), 1);
        assert!(index.errors.contains_key(Path::new("bad.md")));
        assert_eq!(index.note_paths().len(), 2);
        std::fs::write(vault.root.join("good.md"), "updated").unwrap();
        std::fs::write(vault.root.join("bad.md"), "repaired").unwrap();
        index
            .refresh_paths(&vault, ["good.md".into(), "bad.md".into()])
            .unwrap();
        assert!(index.errors.is_empty());
        assert_eq!(index.notes[Path::new("good.md")].text, "updated");
        assert_eq!(index.notes[Path::new("bad.md")].text, "repaired");
        std::fs::write(vault.root.join("bad.md"), [0xff]).unwrap();
        index.refresh_paths(&vault, ["bad.md".into()]).unwrap();
        assert!(!index.notes.contains_key(Path::new("bad.md")));
        std::fs::remove_file(vault.root.join("bad.md")).unwrap();
        index.refresh_paths(&vault, ["bad.md".into()]).unwrap();
        assert!(index.errors.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_task_markers_share_index_and_search_semantics() {
        let source = "- [-] cancelled\n> - [!] important\n1. [✓] complete\n- [ ] open\n\n```\n- [!] code\n```\n\n- [link](url)\n";
        let parsed = parse(source);
        assert_eq!(parsed.tasks.len(), 4);
        assert_eq!(
            parsed
                .tasks
                .iter()
                .map(|t| &source[t.marker.clone()])
                .collect::<Vec<_>>(),
            vec!["-", "!", "✓", " "]
        );
        assert!(parsed.tasks[..3].iter().all(|t| t.checked));
        let changed = set_task(source, parsed.tasks[2].marker.clone(), false).unwrap();
        assert!(changed.contains("1. [ ] complete"));
        let mut index = Index::default();
        index.update("a.md".into(), source.into());
        assert_eq!(index.search("task-done:important").len(), 1);
        assert!(index.search("task-todo:important").is_empty());
        assert!(index.search("task:code").is_empty());
    }
    #[test]
    fn property_search_locates_quoted_keys_and_multiline_values() {
        let mut index = Index::default();
        let source =
            "---\nother: keep\n\"名称: 中文\": done\naliases:\n  - one\n  - two\n---\nbody";
        index.update("note.md".into(), source.into());
        let hits = index.search("[\"名称: 中文\":done]");
        assert_eq!(hits[0].line, 3);
        assert_eq!(hits[0].offset, source.find("\"名称").unwrap());
        assert!(hits[0].excerpt.contains("done"));
        assert!(!hits[0].highlights.is_empty());
        let list = index.search("[aliases:two]");
        assert_eq!(
            list.iter().map(|hit| hit.line).collect::<Vec<_>>(),
            [4, 5, 6]
        );
        assert!(index.search("-[missing]")[0].highlights.is_empty());
    }
    #[test]
    fn property_conditions_use_original_metadata_inside_line_and_section_scopes() {
        let mut index = Index::default();
        let source = "---\nstatus: done\n---\n# Title\nalpha\nbeta\n";
        index.update("note.md".into(), source.into());
        assert_eq!(index.search("[status:done]").len(), 1);
        let line = index.search("line:([status:done] alpha)");
        assert_eq!(line.len(), 1);
        assert_eq!(line[0].line, 5);
        assert_eq!(&line[0].excerpt[line[0].highlights[0].clone()], "alpha");
        assert_eq!(index.search("section:([status:done] Title)")[0].line, 4);
        index.update("note.md".into(), source.replace("done", "draft"));
        assert!(index.search("[status:done]").is_empty());
        assert_eq!(index.search("[status:draft]").len(), 1);
    }
    #[test]
    fn section_search_respects_heading_boundaries_and_nested_descendants() {
        let mut index = Index::default();
        let source = "intro\n# Parent\nalpha\n\nbeta\n## Child\nchildword\n### Deep\ndeepword\n# Other\ngamma\n";
        index.update("note.md".into(), source.into());
        assert_eq!(
            index
                .search("section:(alpha beta)")
                .iter()
                .map(|hit| hit.line)
                .collect::<Vec<_>>(),
            [3, 5]
        );
        assert!(index.search("section:(alpha childword)").is_empty());
        let nested = index.search("section:(Parent section:(match-case:Child section:deepword))");
        assert_eq!(
            nested.iter().map(|hit| hit.line).collect::<Vec<_>>(),
            [2, 6, 9]
        );
        assert_eq!(nested[2].offset, source.find("deepword").unwrap());
        assert_eq!(
            &nested[2].excerpt[nested[2].highlights[0].clone()],
            "deepword"
        );
        assert!(index.search("section:(Other section:childword)").is_empty());
        assert!(index.search("section:(intro section:childword)").is_empty());
    }
    #[test]
    fn block_search_keeps_paragraphs_and_list_items_separate() {
        let mut index = Index::default();
        let source = "alpha\nbeta\n\nalpha\n\nbeta\n\n- item alpha\n  continuation beta\n- item only alpha\n- beta alone\n\n```\nalpha beta\n```\n";
        index.update("note.md".into(), source.into());
        let hits = index.search("block:(alpha beta)");
        assert_eq!(
            hits.iter().map(|hit| hit.line).collect::<Vec<_>>(),
            [1, 2, 8, 9, 14]
        );
        assert_eq!(&hits[3].excerpt[hits[3].highlights[0].clone()], "beta");
        index.update("note.md".into(), "- alpha\n- beta\n".into());
        assert!(index.search("block:(alpha beta)").is_empty());
        index.update("note.md".into(), "> alpha\n>\n> beta\n".into());
        assert_eq!(index.search("block:(alpha beta)").len(), 2);
    }
    #[test]
    fn task_search_filters_state_and_includes_continuations_without_code() {
        let mut index = Index::default();
        let source = "---\ntext: |\n  - [ ] YAMLfake\n---\n- [ ] alpha\n  continuation beta\n\n- [x] alpha beta\n- [?] custom done\n- plain [x] notTask\n\n```md\n- [ ] codefake\n```\n";
        index.update("FilenameOnly.md".into(), source.into());
        let todo = index.search("task-todo:(alpha beta)");
        assert_eq!(todo.iter().map(|hit| hit.line).collect::<Vec<_>>(), [5, 6]);
        assert_eq!(&todo[1].excerpt[todo[1].highlights[0].clone()], "beta");
        assert_eq!(index.search("task-done:alpha").len(), 1);
        assert_eq!(index.search("task-done:custom").len(), 1);
        assert!(index.search("task:codefake").is_empty());
        assert!(index.search("task:YAMLfake").is_empty());
        assert!(index.search("task:notTask").is_empty());
        assert!(index.search("task:FilenameOnly").is_empty());
        assert_eq!(index.search("task:\"\"").len(), 3);
        assert_eq!(index.search("task-todo: \"\"").len(), 1);
        assert_eq!(index.search("task-done:\"\"").len(), 2);
    }
    #[test]
    fn line_scope_requires_same_line_and_preserves_global_offsets() {
        let mut index = Index::default();
        let source = "alpha\r\nbeta\r\n😀alpha beta\r\nalpha skip\r\n";
        index.update("FilenameOnly.md".into(), source.into());
        let hits = index.search("line:(alpha beta)");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 3);
        assert_eq!(hits[0].offset, source.find("😀alpha").unwrap() + "😀".len());
        assert_eq!(
            hits[0]
                .highlights
                .iter()
                .map(|range| &hits[0].excerpt[range.clone()])
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(
            index
                .search("line:(alpha -skip)")
                .iter()
                .map(|hit| hit.line)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            index
                .search("line:-skip")
                .iter()
                .map(|hit| hit.line)
                .collect::<Vec<_>>(),
            [1, 2, 3, 5]
        );
        assert!(index.search("line:FilenameOnly").is_empty());
        assert!(index.search("-line:(alpha beta)").is_empty());
    }
    #[test]
    fn filename_and_path_highlights_follow_successful_query_scopes() {
        let mut index = Index::default();
        let path = PathBuf::from("资料/İdea.md");
        index.update(path.clone(), "body 资料".into());
        let title = path.to_string_lossy().replace('\\', "/");
        let basic = index.search("i");
        assert_eq!(&title[basic[0].title_highlights[0].clone()], "İ");
        let scoped = index.search("(file:İdea OR path:资料) content:body");
        assert_eq!(
            scoped[0]
                .title_highlights
                .iter()
                .map(|range| &title[range.clone()])
                .collect::<Vec<_>>(),
            ["资料", "İdea"]
        );
        assert!(index.search("content:资料")[0].title_highlights.is_empty());
        let excluded = index.search("(file:missing OR path:资料) -content:blocked");
        assert_eq!(excluded[0].title_highlights.len(), 1);
        assert_eq!(&title[excluded[0].title_highlights[0].clone()], "资料");
    }
    #[test]
    fn default_search_includes_filename_while_content_scope_excludes_it() {
        let mut index = Index::default();
        index.update("Meeting.md".into(), "正文\nproject target\n尾行".into());
        assert_eq!(index.search("meeting").len(), 1);
        assert_eq!(index.search("meeting")[0].offset, 0);
        assert!(index.search("content:meeting").is_empty());
        let combined = index.search("meeting project");
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].line, 2);
        assert_eq!(
            &combined[0].excerpt[combined[0].highlights[0].clone()],
            "project"
        );
        assert!(index.search("project -meeting").is_empty());
        assert!(index.search_with_case("meeting", true).unwrap().is_empty());
        assert_eq!(index.search("/Meet.*\\.md/").len(), 1);
    }
    #[test]
    fn grouped_queries_highlight_only_successful_positive_branches() {
        let mut index = Index::default();
        index.update("note.md".into(), "alpha\nblocked\ngamma\ntarget".into());
        let hits = index.search("(alpha -blocked OR gamma) target");
        assert_eq!(hits.iter().map(|hit| hit.line).collect::<Vec<_>>(), [3, 4]);
        assert_eq!(&hits[0].excerpt[hits[0].highlights[0].clone()], "gamma");
        assert_eq!(index.search("-(missing OR absent)").len(), 1);
        assert!(index.search("-(alpha OR gamma)").is_empty());
    }
    #[test]
    fn search_highlights_match_excerpt_bytes_and_merge_overlapping_terms() {
        let mut index = Index::default();
        index.update("note.md".into(), "前😀 Alpha alpha 汉字\nblocked".into());
        let hits = index.search("alpha");
        assert_eq!(
            hits[0]
                .highlights
                .iter()
                .map(|range| &hits[0].excerpt[range.clone()])
                .collect::<Vec<_>>(),
            ["Alpha", "alpha"]
        );
        let overlaps = index.search("alpha OR pha");
        assert_eq!(overlaps[0].highlights.len(), 2);
        let excluded = index.search("alpha -blocked OR 汉字");
        assert_eq!(
            &excluded[0].excerpt[excluded[0].highlights[0].clone()],
            "汉字"
        );
        index.update(
            "note.md".into(),
            format!("{}İx 目标 目标", "前😀".repeat(180)),
        );
        let cropped = index.search("目标");
        assert_eq!(cropped[0].highlights.len(), 2);
        for range in &cropped[0].highlights {
            assert_eq!(&cropped[0].excerpt[range.clone()], "目标");
        }
        let expanded_case = index.search("i");
        assert_eq!(
            &expanded_case[0].excerpt[expanded_case[0].highlights[0].clone()],
            "İ"
        );
    }
    #[test]
    fn search_sorting_happens_before_the_result_limit() {
        use crate::file_order::SortBy;
        let mut index = Index::default();
        for i in 0..210 {
            index.update(format!("note{i}.md").into(), "match".into());
        }
        std::sync::Arc::make_mut(index.notes.get_mut(Path::new("note209.md")).unwrap())
            .times
            .modified = Some(std::time::SystemTime::UNIX_EPOCH);
        let newest = index
            .search_ordered("match", false, SortBy::Modified, true)
            .unwrap();
        assert_eq!(newest.len(), 200);
        assert_eq!(newest[0].path, Path::new("note209.md"));
        let natural = index
            .search_ordered("match", false, SortBy::Name, false)
            .unwrap();
        assert_eq!(natural[2].path, Path::new("note2.md"));
        let reverse = index
            .search_ordered("match", false, SortBy::Name, true)
            .unwrap();
        assert_eq!(reverse[0].path, Path::new("note209.md"));
    }
    #[test]
    fn regex_results_use_multiline_anchors_and_ignore_empty_matches() {
        let mut index = Index::default();
        index.update("note.md".into(), "alpha\nbeta\nalpha\nbeta\n".into());
        assert_eq!(index.search("/^alpha/").len(), 2);
        assert_eq!(
            index
                .search("/(?m)^alpha/")
                .iter()
                .map(|h| h.line)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            index
                .search("/alpha\\nbeta/")
                .iter()
                .map(|h| h.line)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(index.search("/$/").is_empty());
        assert!(index.search("//").is_empty());
        index.update("note.md".into(), "ab a\ncb".into());
        assert_eq!(index.search("/(?s)a.*?b|c/").len(), 1);
    }
    #[test]
    fn cross_line_queries_show_all_terms_and_skip_false_or_branches() {
        let mut index = Index::default();
        let source = "前言\n😀alpha\nblocked\n中文beta\ngamma\n";
        index.update("note.md".into(), source.into());
        let both = index.search("alpha beta");
        assert_eq!(both.iter().map(|hit| hit.line).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(both[0].offset, source.find("alpha").unwrap());
        assert_eq!(both[1].offset, source.find("beta").unwrap());
        let alternatives = index.search("alpha -blocked OR gamma");
        assert_eq!(alternatives.len(), 1);
        assert_eq!(alternatives[0].line, 5);
        assert_eq!(index.search("file:note -absent").len(), 1);
        assert!(index.search("alpha -blocked").is_empty());
        index.update("many.md".into(), "alpha\n".repeat(300));
        assert_eq!(index.search("alpha").len(), 200);
    }
    #[test]
    fn search_excerpts_include_late_matches_and_metadata_hits_are_not_repeated() {
        let mut index = Index::default();
        let text = format!("title\n{}目标\n其他条件\n", "前😀".repeat(160));
        index.update("note.md".into(), text.clone());
        let hits = index.search("目标");
        assert_eq!(hits[0].offset, text.find("目标").unwrap());
        assert!(hits[0].excerpt.contains("目标"));
        let spanning = index.search("目标 其他条件");
        assert_eq!(spanning[0].offset, text.find("目标").unwrap());
        assert_eq!(spanning[0].line, 2);
        assert!(spanning[0].excerpt.contains("目标"));
        assert_eq!(index.search("file:note").len(), 1);
    }
    #[test]
    fn tag_index_keeps_occurrences_separate_from_search_tags() {
        let parsed = parse(
            "---\ntags: [work, work]\nAliases: ['Case Alias']\n---\n#work #work `#work` \\#work",
        );
        assert_eq!(parsed.tags, ["work"]);
        assert_eq!(parsed.tag_counts["work"], 4);
        assert_eq!(parsed.aliases, ["Case Alias"]);
    }
    #[test]
    fn yaml_aliases_with_punctuation_participate_in_search_and_backlinks() {
        let mut index = Index::default();
        index.update(
            "target.md".into(),
            "---\naliases: [\"Smith, John\", 'ÉTUDE']\ntags: ['工作/项目']\n---\n正文".into(),
        );
        index.update("entry.md".into(), "[[Smith, John]] [[étude]]".into());
        assert_eq!(
            index.filenames("Smith, John")[0].path,
            Path::new("target.md")
        );
        assert_eq!(
            index.resolve(Path::new("entry.md"), "étude"),
            Resolution::Found("target.md".into())
        );
        assert_eq!(
            index.backlinks(Path::new("target.md")),
            vec![PathBuf::from("entry.md")]
        );
        assert_eq!(
            index.search("tag:工作/项目")[0].path,
            Path::new("target.md")
        );
    }
    #[test]
    fn filenames_rank_names_and_aliases_and_keep_real_paths() {
        let mut index = Index::default();
        for (path, aliases) in [
            ("a-project-notes.md", "[]"),
            ("b.md", "[project, project plan]"),
            ("c.md", "[project planning]"),
            ("project draft.md", "[]"),
            ("z/project.md", "[]"),
        ] {
            index.update(path.into(), format!("---\naliases: {aliases}\n---\n"));
        }
        let hits = index.filenames("  PROJECT  ");
        assert_eq!(
            hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
            [
                "z/project.md",
                "b.md",
                "project draft.md",
                "c.md",
                "a-project-notes.md"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(hits[1].display_name.as_deref(), Some("project"));
        assert_eq!(hits[3].display_name.as_deref(), Some("project planning"));
        assert!(hits[0].display_name.is_none());
        assert!(
            index
                .search("project")
                .iter()
                .all(|h| h.display_name.is_none())
        );
    }
    #[test]
    fn filenames_limit_after_ranking_and_normalize_queries() {
        let mut index = Index::default();
        for i in 0..250 {
            index.update(format!("a-{i}-目标.md").into(), String::new());
        }
        index.update("资料/目标.md".into(), "---\naliases: [ÉTUDE]\n---\n".into());
        let hits = index.filenames("目标");
        assert_eq!(hits.len(), 200);
        assert_eq!(hits[0].path, Path::new("资料/目标.md"));
        assert_eq!(index.filenames("资料\\目")[0].path, hits[0].path);
        let aliases = index.filenames("étude");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].display_name.as_deref(), Some("ÉTUDE"));
        assert!(index.filenames("不存在").is_empty());
    }
    #[test]
    fn refresh_updates_file_dates_even_when_text_is_unchanged() {
        let root =
            std::env::temp_dir().join(format!("inkstone-index-dates-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "same text").unwrap();
        let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
        let mut index = Index::build(&vault).unwrap();
        let newer =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("note.md"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(newer))
            .unwrap();
        index
            .refresh_paths(&vault, [PathBuf::from("note.md")])
            .unwrap();
        assert_eq!(index.notes[Path::new("note.md")].text, "same text");
        assert_eq!(
            index.notes[Path::new("note.md")].times.modified,
            Some(newer)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fold_ranges_follow_heading_levels_and_nested_blocks() {
        let source = "# A\nbody\n## B\n- parent\n  - child\n# C\n```md\n# literal\n```";
        let parsed = parse(source);
        assert_eq!(parsed.folds, vec![0..5, 2..5, 3..5, 5..9, 6..9]);
        assert_eq!(
            parsed.fold_ranges(true, false),
            vec![0..5, 2..5, 5..9, 6..9]
        );
        assert_eq!(parsed.fold_ranges(false, true), vec![3..5, 6..9]);
        assert!(parsed.fold_ranges(false, false).is_empty());
        assert!(!parsed.folds.iter().any(|r| r.start == 7));
        assert!(parse("# single").folds.is_empty());
        assert_eq!(parse("Title\n=====\nbody").folds, vec![0..3]);
    }
    #[test]
    fn folder_move_updates_incoming_outgoing_and_embedded_assets_once() {
        let root =
            std::env::temp_dir().join(format!("inkstone-folder-references-{}", std::process::id()));
        std::fs::create_dir_all(root.join("原目录/子目录")).unwrap();
        std::fs::write(root.join("原目录/子目录/图 示.svg"), "<svg/>").unwrap();
        let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
        let mut index = Index::default();
        let incoming = "[[原目录/子目录/笔记#标题|显示]]\n![[原目录/子目录/图 示.svg|160]]\n[笔记](原目录/子目录/笔记.md#%E6%A0%87)\n![图](原目录/子目录/图%20示.svg)\n`[[原目录/子目录/笔记]]`\n[外部](https://example.com/a)";
        let outgoing = "[[../同级]] [[外部]] [[#局部]]\n[根](../../外部.md) ![图](图%20示.svg)\n[局部](#标题)\n[引用][r]\n\n[r]: ../../外部.md \"标题\"";
        index.update("入口.md".into(), incoming.into());
        index.update("原目录/子目录/笔记.md".into(), outgoing.into());
        index.update("原目录/同级.md".into(), "# 同级".into());
        index.update("外部.md".into(), "# 外部".into());
        let edits = index.relocation_edits(
            Path::new("原目录"),
            Path::new("归档/新目录"),
            true,
            Some(&vault.root),
        );
        assert_eq!(edits.len(), 2);
        let entry = &edits
            .iter()
            .find(|e| e.0 == Path::new("入口.md"))
            .unwrap()
            .2;
        let entry = percent_encoding::percent_decode_str(entry)
            .decode_utf8()
            .unwrap();
        assert!(entry.contains("[[/归档/新目录/子目录/笔记#标题|显示]]"));
        assert!(entry.contains("![[/归档/新目录/子目录/图 示.svg|160]]"));
        assert!(entry.contains("![图](归档/新目录/子目录/图 示.svg)"));
        assert!(entry.contains("`[[原目录/子目录/笔记]]`"));
        assert!(entry.contains("https://example.com/a"));
        let moved = &edits
            .iter()
            .find(|e| e.0 == Path::new("归档/新目录/子目录/笔记.md"))
            .unwrap()
            .2;
        let moved = percent_encoding::percent_decode_str(moved)
            .decode_utf8()
            .unwrap();
        assert!(moved.contains("[[/归档/新目录/同级]] [[/外部]] [[#局部]]"));
        assert!(moved.contains("[根](../../../外部.md) ![图](图 示.svg)"));
        assert!(moved.contains("[局部](#标题)"));
        assert!(moved.contains("[r]: ../../../外部.md \"标题\""));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tasks_and_block_anchors_are_source_based_and_exclude_code() {
        let source = "# 章节 A\r\n\r\n段落😀 ^para-1\r\n\r\n- [ ] 任务一 ^todo-1\r\n- [X] 任务二\r\n\r\n```md\n- [ ] 假任务 ^fake\n```\n\n^code-1\n\n# 下一节\n";
        let parsed = parse(source);
        assert_eq!(parsed.tasks.len(), 2);
        assert_eq!(&source[parsed.tasks[0].marker.clone()], " ");
        assert!(parsed.tasks[1].checked);
        assert_eq!(
            &source[anchor_range(source, &parsed, "^para-1").unwrap()],
            "段落😀 ^para-1"
        );
        assert!(source[anchor_range(source, &parsed, "^todo-1").unwrap()].starts_with("- [ ]"));
        assert!(source[anchor_range(source, &parsed, "^code-1").unwrap()].starts_with("```"));
        assert!(anchor_range(source, &parsed, "^fake").is_none());
        let section = anchor_range(source, &parsed, "章节-A").unwrap();
        assert!(!source[section].contains("下一节"));
        let changed = set_task(source, parsed.tasks[0].marker.clone(), true).unwrap();
        assert!(changed.contains("- [x] 任务一"));
        assert!(changed.contains("\r\n"));
        assert!(set_task(source, 0..1, true).is_none());
    }
    #[test]
    fn reference_style_links_participate_in_backlinks() {
        let mut index = Index::default();
        index.update("来源.md".into(), "[目标][ref]\n\n[ref]: 目标.md\n".into());
        index.update("目标.md".into(), "".into());
        assert_eq!(
            index.backlinks(Path::new("目标.md")),
            vec![PathBuf::from("来源.md")]
        );
    }
    #[test]
    fn tags_aliases_and_standard_backlinks_ignore_code() {
        let source = "---\naliases: [别名, 'Second']\ntags:\n  - 工作/项目\n---\n# 标题\n\n#中文 #123 #work/sub `#code` \\#escaped\n\n[目标](目标.md#章节)\n\n```md\n#not-a-tag [[目标]]\n```\n";
        let note = parse(source);
        assert_eq!(note.aliases, vec!["别名", "Second"]);
        assert!(note.tags.contains(&"中文".into()));
        assert!(note.tags.contains(&"工作/项目".into()));
        assert!(
            !note
                .tags
                .iter()
                .any(|t| matches!(t.as_str(), "123" | "code" | "escaped" | "not-a-tag"))
        );
        let mut index = Index::default();
        index.update("来源.md".into(), source.into());
        index.update("目标.md".into(), "".into());
        assert_eq!(
            index.backlinks(Path::new("目标.md")),
            vec![PathBuf::from("来源.md")]
        );
        assert_eq!(
            index.resolve(Path::new("目标.md"), "别名"),
            Resolution::Found("来源.md".into())
        );
    }
    #[test]
    fn rename_preserves_aliases_fragments_code_and_relative_destinations() {
        let mut index = Index::default();
        index.update(
            "原目录/笔记.md".into(),
            "[[../目标]] [目标](../目标.md)\n".into(),
        );
        index.update("目标.md".into(), "# 目标".into());
        index.update(
            "来源.md".into(),
            "[[原目录/笔记#章节|显示]]\n[标题](原目录/笔记.md#章节)\n`[[原目录/笔记]]`\n".into(),
        );
        let edits = index.relocation_edits(
            Path::new("原目录/笔记.md"),
            Path::new("新 目录/改名.md"),
            false,
            None,
        );
        let source = &edits
            .iter()
            .find(|(p, _, _)| p == Path::new("来源.md"))
            .unwrap()
            .2;
        assert!(source.contains("[[/新 目录/改名#章节|显示]]"));
        assert!(
            source.contains("[标题](%E6%96%B0%20%E7%9B%AE%E5%BD%95/%E6%94%B9%E5%90%8D.md#章节)")
        );
        assert!(source.contains("`[[原目录/笔记]]`"));
        let moved = &edits
            .iter()
            .find(|(p, _, _)| p == Path::new("新 目录/改名.md"))
            .unwrap()
            .2;
        assert!(moved.contains("[[/目标]]"));
    }
    #[test]
    fn dotted_wiki_names_keep_their_entire_stem() {
        let mut index = Index::default();
        index.update("会议.2026.md".into(), String::new());
        assert_eq!(
            index.resolve(Path::new("来源.md"), "会议.2026"),
            Resolution::Found("会议.2026.md".into())
        );
        assert_eq!(
            index.resolve(Path::new("来源.md"), "项目/会议.2027"),
            Resolution::Missing("项目/会议.2027.md".into())
        );
    }
    #[test]
    fn markdown_paths_are_relative_and_decode_path_separately_from_anchor() {
        let mut index = Index::default();
        index.update("目录/子目录/中文 空格#笔记.md".into(), "# 标题".into());
        index.update("子目录/中文 空格#笔记.md".into(), String::new());
        assert_eq!(index.resolve_markdown(Path::new("目录/来源.md"),"子目录/%E4%B8%AD%E6%96%87%20%E7%A9%BA%E6%A0%BC%23%E7%AC%94%E8%AE%B0.md#%E6%A0%87%E9%A2%98"),
            (Resolution::Found("目录/子目录/中文 空格#笔记.md".into()),Some("标题".into())));
        assert_eq!(
            index
                .resolve_markdown(Path::new("目录/来源.md"), "missing.md")
                .0,
            Resolution::Missing("目录/missing.md".into())
        );
        assert_eq!(
            index.resolve_markdown(Path::new("来源.md"), "image.png").0,
            Resolution::Invalid
        );
        assert_eq!(
            index
                .resolve_markdown(Path::new("来源.md"), "../outside.md")
                .0,
            Resolution::Invalid
        );
    }
    #[test]
    fn ast_links_headings_exclude_code_and_preserve_unicode_ranges() {
        let source = "# 中文😀\n**[[笔记|别名]]** `[[不是]]`\n\n```\n[[也不是]]\n```\n";
        let parsed = parse(source);
        assert_eq!(parsed.headings[0].title, "中文😀");
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(&source[parsed.links[0].range.clone()], "[[笔记|别名]]");
        assert!(
            crate::rendering::reading_document(&Index::default(), Path::new("a.md"), source)
                .markdown
                .contains("[别名](inkstone-reference:0)")
        );
    }
    #[test]
    fn resolution_ambiguity_relative_paths_and_backlink_rebuild() {
        let mut index = Index::default();
        index.update("a/同名.md".into(), "A".into());
        index.update("b/同名.md".into(), "B".into());
        index.update("a/来源.md".into(), "[[同名]] [[b/同名#标题]]".into());
        assert_eq!(
            index.resolve(Path::new("a/来源.md"), "同名"),
            Resolution::Found("a/同名.md".into())
        );
        assert!(matches!(
            index.resolve(Path::new("来源.md"), "同名"),
            Resolution::Ambiguous(_)
        ));
        assert_eq!(
            index.backlinks(Path::new("b/同名.md")),
            vec![PathBuf::from("a/来源.md")]
        );
        index.update("a/来源.md".into(), "无链接".into());
        assert!(index.backlinks(Path::new("b/同名.md")).is_empty());
        assert_eq!(
            index.resolve(Path::new("a/来源.md"), "../新文档"),
            Resolution::Missing("新文档.md".into())
        );
        assert_eq!(
            index.resolve(Path::new("来源.md"), "../越界"),
            Resolution::Invalid
        );
    }
    #[test]
    fn chinese_search_returns_byte_position() {
        let mut index = Index::default();
        index.update("中文.md".into(), "第一行😀\n第二行目标\n".into());
        let hits = index.search("目标");
        assert_eq!(hits[0].offset, "第一行😀\n第二行".len());
        assert_eq!(hits[0].line, 2);
    }
}
