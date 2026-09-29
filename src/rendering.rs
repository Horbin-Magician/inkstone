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
}
impl ReadingDocument {
    pub fn source_matches(&self, path: &Path, text: &str) -> bool {
        self.sources.get(path).is_some_and(|s| s.as_ref() == text)
    }
    pub fn output_offset(&self, path: &Path, source: usize) -> Option<usize> {
        if let Some(m) = self
            .locations
            .iter()
            .find(|m| m.path == path && m.start <= source && source <= m.start + m.output.len())
        {
            return Some(m.output.start + source - m.start);
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
}
enum Action {
    Wiki(index::WikiLink, bool),
    Destination(String),
    Remove,
    Literal(String),
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
            let start = self.output.markdown.len();
            self.output.markdown.push_str(part);
            if let Some((path, source)) = origin {
                self.maps.push(SourceMap {
                    output: start..self.output.markdown.len(),
                    path: path.to_path_buf(),
                    start: source + offset,
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
        let parsed = index::parse(&source);
        let mut actions = vec![];
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
        for span in crate::markdown::spans(&source)
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
    };
    builder.note(path, Arc::from(source), 0..source.len());
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
    use super::*;
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
