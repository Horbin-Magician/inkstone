//! Local link diagnostics; never requests remote URLs.
use crate::{
    index::{self, Index, Resolution},
    rendering::{self, Reference},
    syntax::Snapshot,
};
use markdown_parser::mdast::Node;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Issue {
    pub from: PathBuf,
    pub offset: usize,
    pub target: String,
    pub reason: String,
}
pub fn scan(root: &Path, index: &Index) -> Vec<Issue> {
    let mut issues = vec![];
    for (from, note) in &index.notes {
        let mut references: Vec<_> = note
            .parsed
            .links
            .iter()
            .map(|l| (l.range.start, l.target.clone(), true))
            .collect();
        fn walk(node: &Node, refs: &mut Vec<(usize, String, bool)>) {
            let target = match node {
                Node::Link(n) => Some(&n.url),
                Node::Image(n) => Some(&n.url),
                Node::Definition(n) => Some(&n.url),
                _ => None,
            };
            if let Some(target) = target {
                refs.push((
                    node.position().map_or(0, |p| p.start.offset),
                    target.clone(),
                    false,
                ));
            }
            if let Some(children) = node.children() {
                for n in children {
                    walk(n, refs);
                }
            }
        }
        let snapshot = Snapshot::new(&note.text);
        if let Some(ast) = &snapshot.ast {
            walk(ast, &mut references);
            references.extend(
                crate::syntax::html_destinations(ast, &snapshot.structural)
                    .into_iter()
                    .map(|(r, t)| (r.start, t, false)),
            );
        }
        let mut seen = BTreeSet::new();
        for (offset, target, wiki) in references {
            if rendering::is_external_link(&target)
                || rendering::is_remote_image(&target)
                || !seen.insert((offset, target.clone()))
            {
                continue;
            }
            let (resolution, fragment) = if wiki {
                (
                    index.resolve(from, &target),
                    target.split_once('#').map(|(_, f)| f.to_owned()),
                )
            } else {
                index.resolve_markdown(from, &target)
            };
            let reason = match resolution {
                Resolution::Found(path) if index.notes.contains_key(&path) => {
                    fragment.filter(|f| !f.is_empty()).and_then(|fragment| {
                        let n = &index.notes[&path];
                        index::anchor_range(&n.text, &n.parsed, &fragment)
                            .is_none()
                            .then(|| "标题或块引用不存在".to_string())
                    })
                }
                Resolution::Ambiguous(paths) => {
                    Some(format!("存在 {} 个同名目标，请使用完整路径", paths.len()))
                }
                _ => {
                    if rendering::asset_path(
                        root,
                        &Reference {
                            from: from.clone(),
                            target: target.clone(),
                            wiki,
                        },
                        &index.files,
                    )
                    .is_some()
                    {
                        None
                    } else {
                        Some("目标不存在、无法读取或路径无效".into())
                    }
                }
            };
            if let Some(reason) = reason {
                issues.push(Issue {
                    from: from.clone(),
                    offset,
                    target,
                    reason,
                });
            }
        }
    }
    for (from, error) in &index.errors {
        issues.push(Issue {
            from: from.clone(),
            offset: 0,
            target: String::new(),
            reason: format!("笔记无法读取：{error}"),
        });
    }
    issues.sort_by(|a, b| a.from.cmp(&b.from).then(a.offset.cmp(&b.offset)));
    issues
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_missing_ambiguous_and_bad_anchors_but_skips_external_and_code() {
        let mut index = Index::default();
        index.update("a.md".into(),"[[missing]] [[x]] [[b#不存在]] [[b#标题]] [[b#^block]]\n\n[坏](b.md#错误) [外部](https://example.com) `[[code]]`\n".into());
        index.update("b.md".into(), "# 标题\n\nblock ^block\n".into());
        index.update("one/x.md".into(), "x".into());
        index.update("two/x.md".into(), "x".into());
        let issues = scan(Path::new("/nonexistent"), &index);
        assert_eq!(issues.len(), 4, "{issues:?}");
        assert!(issues.iter().any(|i| i.reason.contains("同名")));
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.reason.contains("标题或块"))
                .count(),
            2
        );
    }
}
