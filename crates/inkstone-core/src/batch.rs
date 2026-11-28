//! Reviewed bulk edits. Planning never writes; applying checks every baseline first.
use crate::{index::Index, vault::Vault};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub find: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
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
    let regex = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!spec.case_sensitive)
        .size_limit(crate::search::REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut result = Plan::default();
    for (path, note) in &index.notes {
        if !folder.is_empty() && !path.starts_with(folder_path) {
            continue;
        }
        let transformed = {
            let count = regex
                .find_iter(&note.text)
                .try_fold(0usize, |count, found| {
                    (!found.is_empty()).then_some(count + 1)
                });
            if let Some(count) = count {
                if count == 0 {
                    continue;
                }
                let after = if spec.regex {
                    regex.replace_all(&note.text, spec.replacement.as_str())
                } else {
                    regex.replace_all(&note.text, regex::NoExpand(&spec.replacement))
                };
                Ok((after, count))
            } else {
                Err("不支持零宽匹配，请使用匹配具体字符的表达式。")
            }
        };
        match transformed {
            Ok((after, count)) if after != note.text => result.edits.push(Edit {
                path: path.clone(),
                before: note.text.clone(),
                after: after.into_owned(),
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
            folder: String::new(),
        }
    }
    #[test]
    fn batch_and_search_reject_oversized_compiled_regexes() {
        let pattern = r"\w{100}";
        // This fits the regex crate's default budget but exceeds our shared budget.
        assert!(regex::Regex::new(pattern).is_ok());
        let mut s = spec();
        s.regex = true;
        s.find = pattern.into();
        assert!(plan(&Index::default(), &s).is_err());
        assert!(crate::search::Query::parse(&format!("/{pattern}/")).is_err());
    }
    #[test]
    fn dense_replacements_count_every_match_and_reject_late_empty_matches() {
        let mut index = Index::default();
        index.update("dense.md".into(), "old ".repeat(10_000));
        let mut s = spec();
        let result = plan(&index, &s).unwrap();
        assert_eq!(result.edits.len(), 1);
        assert_eq!(result.edits[0].count, 10_000);
        assert_eq!(result.edits[0].after, "新😀$1 ".repeat(10_000));
        index.update("dense.md".into(), "old tail".into());
        s.regex = true;
        s.find = "old|$".into();
        let result = plan(&index, &s).unwrap();
        assert!(result.edits.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("零宽"));
    }
    #[test]
    fn literal_regex_scope_preserve_source_boundaries() {
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
