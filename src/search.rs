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
impl Pattern {
    fn visit_offsets(&self, text: &str, mut visit: impl FnMut(usize) -> bool) {
        match self {
            Self::Regex(regex) => {
                for found in regex.find_iter(text) {
                    if !found.is_empty() && !visit(found.start()) {
                        break;
                    }
                }
            }
            Self::Text(needle) => {
                let lower = text.to_lowercase();
                let mut chars = text.char_indices();
                let (mut lower_end, mut original) = (0, 0);
                for (offset, _) in lower.match_indices(needle) {
                    while lower_end <= offset {
                        let Some((start, ch)) = chars.next() else {
                            return;
                        };
                        original = start;
                        lower_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
                    }
                    if !visit(original) {
                        break;
                    }
                }
            }
        }
    }
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Text(s) => text.to_lowercase().contains(s),
            Self::Regex(r) => r.find_iter(text).any(|found| !found.is_empty()),
        }
    }
    fn first_offset(&self, text: &str) -> Option<usize> {
        match self {
            Self::Regex(regex) => regex
                .find_iter(text)
                .find(|found| !found.is_empty())
                .map(|m| m.start()),
            Self::Text(needle) => {
                let offset = text.to_lowercase().find(needle)?;
                let mut lower_offset = 0;
                for (original, ch) in text.char_indices() {
                    lower_offset += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
                    if lower_offset > offset {
                        return Some(original);
                    }
                }
                None
            }
        }
    }
}
impl Term {
    fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        let found = match self.field {
            Field::Text => self.pattern.matches(text),
            Field::File => self
                .pattern
                .matches(&path.file_name().unwrap_or_default().to_string_lossy()),
            Field::Path => self
                .pattern
                .matches(&path.to_string_lossy().replace('\\', "/")),
            Field::Tag => tags.iter().any(|tag| match &self.pattern {
                Pattern::Text(s) => {
                    tag.to_lowercase() == *s || tag.to_lowercase().starts_with(&format!("{s}/"))
                }
                Pattern::Regex(r) => r.find_iter(tag).any(|found| !found.is_empty()),
            }),
        };
        found != self.exclude
    }
}
pub struct Query {
    groups: Vec<Vec<Term>>,
}
pub struct LineMatch {
    pub offset: usize,
    pub line: usize,
    pub range: std::ops::Range<usize>,
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
                        .multi_line(true)
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
        self.groups.iter().any(|group| {
            !group.is_empty() && group.iter().all(|term| term.matches(path, text, tags))
        })
    }
    pub fn first_offset(&self, path: &Path, text: &str, tags: &[String]) -> Option<usize> {
        self.groups
            .iter()
            .filter(|group| group.iter().all(|term| term.matches(path, text, tags)))
            .flat_map(|group| group.iter())
            .filter(|term| !term.exclude && matches!(term.field, Field::Text))
            .filter_map(|term| term.pattern.first_offset(text))
            .min()
    }
    /// Only terms from Boolean branches satisfied by the whole document may produce hits.
    pub fn matching_lines(
        &self,
        path: &Path,
        text: &str,
        tags: &[String],
        limit: usize,
    ) -> Vec<LineMatch> {
        let patterns: Vec<_> = self
            .groups
            .iter()
            .filter(|group| group.iter().all(|term| term.matches(path, text, tags)))
            .flat_map(|group| group.iter())
            .filter(|term| !term.exclude && matches!(term.field, Field::Text))
            .map(|term| &term.pattern)
            .collect();
        if patterns.is_empty() || limit == 0 {
            return vec![];
        }
        let starts: Vec<_> = std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter_map(|(i, byte)| (byte == b'\n').then_some(i + 1)),
            )
            .collect();
        let mut matches = std::collections::BTreeMap::<usize, usize>::new();
        for pattern in patterns {
            pattern.visit_offsets(text, |offset| {
                let line = starts.partition_point(|start| *start <= offset) - 1;
                if matches.len() == limit
                    && matches
                        .last_key_value()
                        .is_some_and(|(last, _)| line > *last)
                {
                    return false;
                }
                matches
                    .entry(line)
                    .and_modify(|old| *old = (*old).min(offset))
                    .or_insert(offset);
                if matches.len() > limit {
                    matches.pop_last();
                }
                true
            });
        }
        matches
            .into_iter()
            .map(|(line, offset)| LineMatch {
                offset,
                line: line + 1,
                range: starts[line]..starts.get(line + 1).copied().unwrap_or(text.len()),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_matching_limit_counts_lines_and_keeps_earliest_across_terms() {
        let text = format!("early İx\n{}\nlate\n", "word ".repeat(300));
        let query = Query::parse("late OR x OR word").unwrap();
        let matches = query.matching_lines(Path::new("note"), &text, &[], 2);
        assert_eq!(matches.iter().map(|m| m.line).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(matches[0].offset, "early İ".len());
        assert_eq!(
            Query::parse("word OR late")
                .unwrap()
                .matching_lines(Path::new("note"), &text, &[], 2)
                .len(),
            2
        );
    }
    #[test]
    fn first_match_uses_original_bytes_and_matching_boolean_branches() {
        let path = Path::new("note.md");
        assert_eq!(
            Query::parse("目标")
                .unwrap()
                .first_offset(path, "😀前缀目标", &[]),
            Some("😀前缀".len())
        );
        assert_eq!(
            Query::parse("x").unwrap().first_offset(path, "İx", &[]),
            Some("İ".len())
        );
        assert_eq!(
            Query::parse("i").unwrap().first_offset(path, "İx", &[]),
            Some(0)
        );
        assert_eq!(
            Query::parse("/目标\\d+/ OR missing")
                .unwrap()
                .first_offset(path, "前缀目标42", &[]),
            Some("前缀".len())
        );
        assert_eq!(
            Query::parse("absent prefix OR real")
                .unwrap()
                .first_offset(path, "prefix real", &[]),
            Some(7)
        );
        assert_eq!(
            Query::parse("file:note -absent")
                .unwrap()
                .first_offset(path, "body", &[]),
            None
        );
    }
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
