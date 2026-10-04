//! Reviewed bulk edits. Planning never writes; applying checks every baseline first.
use crate::{index::Index, vault::Vault};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub find: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub tags: bool,
    pub descendants: bool,
    pub folder: String,
}
#[derive(Clone, Debug)]
pub struct Edit {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub count: usize,
}
#[derive(Default)]
pub struct Plan {
    pub edits: Vec<Edit>,
    pub warnings: Vec<String>,
}
#[derive(Default)]
pub struct Applied {
    pub written: Vec<Edit>,
    pub errors: Vec<String>,
}
fn rename_tag(value: &str, old: &str, new: &str, descendants: bool) -> Option<String> {
    let trimmed = value.trim();
    let tag = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let lower = tag.to_lowercase();
    let old = old.to_lowercase();
    let replacement = if lower == old {
        new.to_owned()
    } else if descendants && lower.starts_with(&format!("{old}/")) {
        // Find component boundary in the original UTF-8 spelling (case folding changes byte sizes).
        let n = old.split('/').count();
        let suffix = tag.split('/').skip(n).collect::<Vec<_>>().join("/");
        format!("{new}/{suffix}")
    } else {
        return None;
    };
    Some(if trimmed.starts_with('#') {
        format!("#{replacement}")
    } else {
        replacement
    })
}
fn tag_edit(source: &str, spec: &Spec) -> Result<(String, usize), String> {
    let old = spec.find.trim().trim_start_matches('#');
    let new = spec.replacement.trim().trim_start_matches('#');
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
            && s.chars().any(|c| !c.is_numeric())
            && s.split('/').all(|s| !s.is_empty())
    };
    if !valid(old) || !valid(new) {
        return Err("标签需包含有效文字，不能有空层级；目标标签不能为空。".into());
    }
    let mut edits = vec![];
    let mut count = 0;
    for (range, tag) in crate::index::parse(source).inline_tags {
        if let Some(new) = rename_tag(&tag, old, new, spec.descendants)
            && new != tag
        {
            edits.push((range, new));
            count += 1;
        }
    }
    if let Some(block) = crate::properties::block(source) {
        let start = source.find('\n').unwrap() + 1;
        let end = source[..block.end]
            .trim_end_matches(['\r', '\n'])
            .rfind('\n')
            .unwrap()
            + 1;
        let yaml = &source[start..end];
        let (_, _, entries) = crate::yaml_source::entries(yaml)?;
        for entry in entries
            .into_iter()
            .filter(|e| e.key.eq_ignore_ascii_case("tags") || e.key.eq_ignore_ascii_case("tag"))
        {
            if yaml[entry.start..entry.value.start].contains('&') {
                return Err(
                    "标签属性使用 YAML 锚点，请先在源码中处理，避免影响其他属性引用。".into(),
                );
            }
            let raw = &yaml[entry.value.clone()];
            let mut value: serde_json::Value =
                serde_saphyr::from_str(raw).map_err(|e| format!("标签属性无法安全处理：{e}"))?;
            let mut changed = 0;
            let mut replace = |v: &mut serde_json::Value| {
                if let Some(s) = v.as_str()
                    && let Some(new) = rename_tag(s, old, new, spec.descendants)
                    && new != s
                {
                    *v = serde_json::Value::String(new);
                    changed += 1;
                }
            };
            if let Some(array) = value.as_array_mut() {
                for v in array {
                    replace(v);
                }
            } else {
                replace(&mut value);
            }
            if changed > 0 {
                if let Some(array) = value.as_array_mut() {
                    let mut seen = std::collections::BTreeSet::new();
                    array.retain(|v| {
                        let Some(s) = v.as_str() else {
                            return true;
                        };
                        if rename_tag(s, new, new, spec.descendants).is_none() {
                            return true;
                        }
                        seen.insert(s.trim().trim_start_matches('#').to_lowercase())
                    });
                }
                edits.push((
                    start + entry.value.start..start + entry.value.end,
                    value.to_string(),
                ));
                count += changed;
            }
        }
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = source.to_owned();
    for (r, value) in edits.into_iter().rev() {
        out.replace_range(r, &value);
    }
    if let Some(block) = crate::properties::block(&out) {
        let start = out.find('\n').unwrap() + 1;
        let end = out[..block.end]
            .trim_end_matches(['\r', '\n'])
            .rfind('\n')
            .unwrap()
            + 1;
        serde_saphyr::from_str::<serde_json::Value>(&out[start..end])
            .map_err(|_| "修改会破坏 YAML，请先在源码中处理属性引用。".to_string())?;
    }
    Ok((out, count))
}
pub fn plan(index: &Index, spec: &Spec) -> Result<Plan, String> {
    if spec.find.is_empty() {
        return Err("查找内容不能为空。".into());
    }
    let folder = spec.folder.trim().trim_end_matches('/');
    let folder_path = Path::new(folder);
    if !folder.is_empty()
        && folder_path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("范围应为库内相对文件夹路径。".into());
    }
    let pattern = if spec.regex {
        spec.find.clone()
    } else {
        regex::escape(&spec.find)
    };
    let regex = if spec.tags {
        None
    } else {
        Some(
            regex::RegexBuilder::new(&pattern)
                .case_insensitive(!spec.case_sensitive)
                .build()
                .map_err(|e| e.to_string())?,
        )
    };
    let mut result = Plan::default();
    for (path, note) in &index.notes {
        if !folder.is_empty() && !path.starts_with(folder_path) {
            continue;
        }
        let transformed = if spec.tags {
            tag_edit(&note.text, spec)
        } else {
            let regex = regex.as_ref().unwrap();
            let matches: Vec<_> = regex.find_iter(&note.text).collect();
            if matches.iter().any(|m| m.is_empty()) {
                Err("不支持零宽匹配，请使用匹配具体字符的表达式。".into())
            } else {
                let after = if spec.regex {
                    regex.replace_all(&note.text, spec.replacement.as_str())
                } else {
                    regex.replace_all(&note.text, regex::NoExpand(&spec.replacement))
                };
                Ok((after.into_owned(), matches.len()))
            }
        };
        match transformed {
            Ok((after, count)) if after != note.text => result.edits.push(Edit {
                path: path.clone(),
                before: note.text.clone(),
                after,
                count,
            }),
            Ok(_) => (),
            Err(error) => result.warnings.push(format!("{}：{error}", path.display())),
        }
    }
    result.warnings.extend(
        index
            .errors
            .iter()
            .filter(|(p, _)| folder.is_empty() || p.starts_with(folder_path))
            .map(|(p, e)| format!("{}：{e}", p.display())),
    );
    Ok(result)
}
pub fn apply(vault: &Vault, edits: Vec<Edit>) -> Applied {
    let mut result = Applied::default();
    for e in &edits {
        match vault.read(&e.path) {
            Ok(Some(text)) if text == e.before => (),
            _ => result.errors.push(format!(
                "{} 在预览后已变化或无法读取，请重新预览。",
                e.path.display()
            )),
        }
    }
    if !result.errors.is_empty() {
        return result;
    }
    for e in edits {
        match vault.save(&e.path, Some(&e.before), &e.after) {
            Ok(_) => result.written.push(e),
            Err(error) => result.errors.push(format!("{}：{error}", e.path.display())),
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> Spec {
        Spec {
            find: "old".into(),
            replacement: "新😀$1".into(),
            regex: false,
            case_sensitive: false,
            tags: false,
            descendants: true,
            folder: String::new(),
        }
    }
    #[test]
    fn literal_regex_scope_and_tag_renaming_preserve_source_boundaries() {
        let mut i = Index::default();
        i.update("folder/a.md".into(), "OLD\r\nold".into());
        i.update("outside.md".into(), "old".into());
        let mut s = spec();
        s.folder = "folder".into();
        let p = plan(&i, &s).unwrap();
        assert_eq!(p.edits.len(), 1);
        assert_eq!(p.edits[0].after, "新😀$1\r\n新😀$1");
        s.regex = true;
        s.find = "(old)".into();
        s.replacement = "$1-new".into();
        assert_eq!(plan(&i, &s).unwrap().edits[0].after, "OLD-new\r\nold-new");
        s.find = "^".into();
        let p = plan(&i, &s).unwrap();
        assert!(p.edits.is_empty() && !p.warnings.is_empty());
        i = Index::default();
        let text = "---\r\nTags: ['Work', 'work/sub'] # keep\r\nother: yes\r\n---\r\n#work #work/sub #worker `#work` \\#work\r\n```md\r\n#work\r\n```\r\n";
        i.update("a.md".into(), text.into());
        s = spec();
        s.tags = true;
        s.find = "work".into();
        s.replacement = "项目".into();
        let p = plan(&i, &s).unwrap();
        assert_eq!(p.edits[0].count, 4);
        let after = &p.edits[0].after;
        assert!(after.contains("#项目 #项目/sub #worker `#work` \\#work"));
        assert!(after.contains("# keep\r\nother: yes"));
        assert!(after.contains("```md\r\n#work\r\n```"));
        assert_eq!(
            crate::index::parse(after).tags,
            vec!["worker", "项目", "项目/sub"]
        );
    }
    #[test]
    fn tag_merge_deduplicates_targets_and_rejects_broken_yaml_aliases() {
        let mut index = Index::default();
        index.update(
            "a.md".into(),
            "---\ntags: [old, done, other, other]\n---\n#old\n".into(),
        );
        let mut s = spec();
        s.tags = true;
        s.replacement = "done".into();
        let p = plan(&index, &s).unwrap();
        assert!(p.edits[0].after.contains("[\"done\",\"other\",\"other\"]"));
        index.update(
            "a.md".into(),
            "---\ntags: &shared [old]\nother: *shared\n---\n#old\n".into(),
        );
        let p = plan(&index, &s).unwrap();
        assert!(p.edits.is_empty());
        assert!(!p.warnings.is_empty());
    }
    #[test]
    fn apply_checks_all_baselines_before_writing_and_keeps_history() {
        let root = std::env::temp_dir().join(format!(
            "inkstone-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let v = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for name in ["a.md", "b.md"] {
            std::fs::write(v.root.join(name), "old").unwrap();
        }
        let p = plan(&Index::build(&v).unwrap(), &spec()).unwrap();
        std::fs::write(v.root.join("b.md"), "external").unwrap();
        let r = apply(&v, p.edits);
        assert!(r.written.is_empty() && !r.errors.is_empty());
        assert_eq!(v.read(Path::new("a.md")).unwrap().unwrap(), "old");
        let p = plan(&Index::build(&v).unwrap(), &spec()).unwrap();
        let r = apply(&v, p.edits);
        assert_eq!(r.written.len(), 1);
        let h = v.history(Path::new("a.md")).unwrap();
        let record = v.read_history(Path::new("a.md"), &h[0].journal).unwrap();
        assert_eq!(record.baseline.as_deref(), Some("old"));
        assert_eq!(record.draft, "新😀$1");
        std::fs::remove_dir_all(root).unwrap();
    }
}
