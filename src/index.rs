use crate::vault::{Vault, VaultError};
use markdown_parser::mdast::Node;
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
#[derive(Clone, Debug, Default)]
pub struct ParsedNote {
    pub links: Vec<WikiLink>,
    pub headings: Vec<Heading>,
}

pub fn parse(source: &str) -> ParsedNote {
    fn label(node: &Node) -> String {
        match node {
            Node::Text(n) => n.value.clone(),
            Node::InlineCode(n) => n.value.clone(),
            _ => node
                .children()
                .map(|c| c.iter().map(label).collect())
                .unwrap_or_default(),
        }
    }
    fn walk(node: &Node, source: &str, result: &mut ParsedNote) {
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
                let mut offset = 0;
                while let Some(start) = raw[offset..].find("[[") {
                    let start = offset + start;
                    let preceding = raw[..start]
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
                        if !target.trim().is_empty() {
                            result.links.push(WikiLink {
                                range: range.start + start..range.start + end + 2,
                                target: target.trim().into(),
                                label: label.into(),
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
    let mut result = ParsedNote::default();
    if let Ok(root) = markdown_parser::to_mdast(source, &markdown_parser::ParseOptions::gfm()) {
        walk(&root, source, &mut result);
    }
    result
}

#[derive(Clone, Debug)]
pub struct IndexedNote {
    pub text: String,
    pub parsed: ParsedNote,
}
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub notes: BTreeMap<PathBuf, IndexedNote>,
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
    pub offset: usize,
    pub line: usize,
    pub excerpt: String,
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
    pub fn build(vault: &Vault) -> Result<Self, VaultError> {
        let mut index = Self::default();
        for path in vault.scan()? {
            if let Some(text) = vault.read(&path)? {
                index.update(path, text);
            }
        }
        Ok(index)
    }
    pub fn update(&mut self, path: PathBuf, text: String) {
        let parsed = parse(&text);
        self.notes.insert(path, IndexedNote { text, parsed });
    }
    pub fn refresh_paths(
        &mut self,
        vault: &Vault,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<(), VaultError> {
        for path in paths {
            match vault.read(&path)? {
                Some(text) => {
                    if self.notes.get(&path).is_none_or(|n| n.text != text) {
                        self.update(path, text);
                    }
                }
                None => {
                    self.notes.remove(&path);
                }
            }
        }
        Ok(())
    }
    pub fn resolve(&self, from: &Path, target: &str) -> Resolution {
        let target = target
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .replace('\\', "/");
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
            path.set_extension("md");
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
            0 => Resolution::Missing(path),
            1 => Resolution::Found(matches[0].clone()),
            _ => Resolution::Ambiguous(matches),
        }
    }
    pub fn filenames(&self, query: &str) -> Vec<PathBuf> {
        let query = query.to_lowercase();
        self.notes
            .keys()
            .filter(|p| key(p).contains(&query))
            .take(200)
            .cloned()
            .collect()
    }
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        if query.trim().is_empty() {
            return vec![];
        }
        let query = query.to_lowercase();
        let mut hits = vec![];
        for (path, note) in &self.notes {
            let mut offset = 0;
            for (line, text) in note.text.split_inclusive('\n').enumerate() {
                if text.to_lowercase().contains(&query) {
                    hits.push(SearchHit {
                        path: path.clone(),
                        offset,
                        line: line + 1,
                        excerpt: text.chars().take(100).collect(),
                    });
                    if hits.len() == 200 {
                        return hits;
                    }
                }
                offset += text.len();
            }
        }
        hits
    }
    pub fn backlinks(&self, target: &Path) -> Vec<PathBuf> {
        self.notes.iter().filter(|(from,note)|note.parsed.links.iter().any(|l|matches!(self.resolve(from,&l.target),Resolution::Found(ref p)if key(p)==key(target)))).map(|(p,_)|p.clone()).collect()
    }
}

/// Read-only display transformation. The editor's source remains untouched.
pub fn reading_source(source: &str, parsed: &ParsedNote) -> String {
    let mut result = String::new();
    let mut offset = 0;
    for (index, link) in parsed.links.iter().enumerate() {
        result.push_str(&source[offset..link.range.start]);
        let label = link
            .label
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]");
        result.push_str(&format!("[{label}](inkstone-link:{index})"));
        offset = link.range.end;
    }
    result.push_str(&source[offset..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ast_links_headings_exclude_code_and_preserve_unicode_ranges() {
        let source = "# 中文😀\n**[[笔记|别名]]** `[[不是]]`\n\n```\n[[也不是]]\n```\n";
        let parsed = parse(source);
        assert_eq!(parsed.headings[0].title, "中文😀");
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(&source[parsed.links[0].range.clone()], "[[笔记|别名]]");
        assert!(reading_source(source, &parsed).contains("[别名](inkstone-link:0)"));
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
        assert_eq!(hits[0].offset, "第一行😀\n".len());
        assert_eq!(hits[0].line, 2);
    }
}
