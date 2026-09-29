//! Obsidian-style text terms, quoted phrases, OR, exclusions and field filters.
use regex::{Regex, RegexBuilder};
use std::path::Path;

enum Pattern {
    Text(String),
    Regex(Regex),
}
enum Field {
    Text,
    File,
    Path,
    Tag,
}
struct Term {
    field: Field,
    pattern: Pattern,
    exclude: bool,
}
pub struct Query {
    groups: Vec<Vec<Term>>,
}

impl Query {
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut words = vec![];
        let mut current = String::new();
        let mut quote = false;
        let mut regex = false;
        let mut escape = false;
        for ch in input.chars() {
            if escape {
                current.push(ch);
                escape = false;
                continue;
            }
            if ch == '\\' {
                current.push(ch);
                escape = true;
                continue;
            }
            if ch == '"' && !regex {
                quote = !quote;
                continue;
            }
            if ch == '/' && !quote && (regex || current.is_empty() || current == "-") {
                regex = !regex;
                current.push(ch);
                continue;
            }
            if ch.is_whitespace() && !quote && !regex {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            } else {
                current.push(ch);
            }
        }
        if quote || regex {
            return Err("搜索表达式中有未闭合的引号或正则表达式".into());
        }
        if !current.is_empty() {
            words.push(current);
        }
        let mut groups = vec![vec![]];
        for word in words {
            if word == "OR" {
                if groups.last().is_some_and(Vec::is_empty) {
                    return Err("OR 前缺少搜索条件".into());
                }
                groups.push(vec![]);
                continue;
            }
            let (exclude, word) = word
                .strip_prefix('-')
                .map_or((false, word.as_str()), |s| (true, s));
            let (field, value) = if let Some(s) = word.strip_prefix("file:") {
                (Field::File, s)
            } else if let Some(s) = word.strip_prefix("path:") {
                (Field::Path, s)
            } else if let Some(s) = word.strip_prefix("tag:") {
                (Field::Tag, s.trim_start_matches('#'))
            } else {
                (Field::Text, word)
            };
            if value.is_empty() {
                return Err("搜索条件不能为空".into());
            }
            let pattern = if value.starts_with('/') && value.ends_with('/') && value.len() > 1 {
                Pattern::Regex(
                    RegexBuilder::new(&value[1..value.len() - 1])
                        .case_insensitive(true)
                        .size_limit(2 * 1024 * 1024)
                        .build()
                        .map_err(|e| format!("无效的正则表达式：{e}"))?,
                )
            } else {
                Pattern::Text(value.to_lowercase())
            };
            groups.last_mut().unwrap().push(Term {
                field,
                pattern,
                exclude,
            });
        }
        if groups.len() > 1 && groups.last().is_some_and(Vec::is_empty) {
            return Err("OR 后缺少搜索条件".into());
        }
        Ok(Self { groups })
    }
    pub fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        fn matches(pattern: &Pattern, text: &str) -> bool {
            match pattern {
                Pattern::Text(s) => text.to_lowercase().contains(s),
                Pattern::Regex(r) => r.is_match(text),
            }
        }
        self.groups.iter().any(|group| {
            !group.is_empty()
                && group.iter().all(|term| {
                    let found = match term.field {
                        Field::Text => matches(&term.pattern, text),
                        Field::File => matches(
                            &term.pattern,
                            &path.file_name().unwrap_or_default().to_string_lossy(),
                        ),
                        Field::Path => {
                            matches(&term.pattern, &path.to_string_lossy().replace('\\', "/"))
                        }
                        Field::Tag => tags.iter().any(|tag| match &term.pattern {
                            Pattern::Text(s) => {
                                tag.to_lowercase() == *s
                                    || tag.to_lowercase().starts_with(&format!("{s}/"))
                            }
                            Pattern::Regex(r) => r.is_match(tag),
                        }),
                    };
                    found != term.exclude
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrases_paths_tags_boolean_and_regex() {
        let p = Path::new("项目/会议.md");
        let tags = vec!["工作/会议".into()];
        assert!(
            Query::parse("path:项目 tag:工作 \"hello world\" -取消")
                .unwrap()
                .matches(p, "hello world 今天", &tags)
        );
        assert!(
            !Query::parse("file:日记 OR tag:私人")
                .unwrap()
                .matches(p, "hello world", &tags)
        );
        assert!(
            Query::parse("/会议\\d+/ OR file:会议")
                .unwrap()
                .matches(p, "", &tags)
        );
        assert!(Query::parse("/[invalid/").is_err());
        assert!(Query::parse("\"unclosed").is_err());
        assert!(Query::parse("word OR").is_err());
    }
}
