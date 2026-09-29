//! Obsidian-style text terms, quoted phrases, OR, exclusions and field filters.
use regex::{Regex, RegexBuilder};
use std::path::Path;
mod group;
mod task;

enum Pattern {
    Text(String),
    Exact(String),
    Regex(Regex),
}
enum Field {
    Text,
    Content,
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
    fn visit_ranges(&self, text: &str, mut visit: impl FnMut(std::ops::Range<usize>) -> bool) {
        match self {
            Self::Exact(needle) => {
                for (offset, _) in text.match_indices(needle) {
                    if !visit(offset..offset + needle.len()) {
                        break;
                    }
                }
            }
            Self::Regex(regex) => {
                for found in regex.find_iter(text) {
                    if !found.is_empty() && !visit(found.range()) {
                        break;
                    }
                }
            }
            Self::Text(needle) => {
                let lower = text.to_lowercase();
                let mut chars = text.char_indices();
                let (mut lower_end, mut original, mut original_end) = (0, 0, 0);
                for (offset, _) in lower.match_indices(needle) {
                    while lower_end <= offset {
                        let Some((start, ch)) = chars.next() else {
                            return;
                        };
                        original = start;
                        original_end = start + ch.len_utf8();
                        lower_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
                    }
                    let match_start = original;
                    while lower_end < offset + needle.len() {
                        let Some((start, ch)) = chars.next() else {
                            return;
                        };
                        original = start;
                        original_end = start + ch.len_utf8();
                        lower_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
                    }
                    if !visit(match_start..original_end) {
                        break;
                    }
                }
            }
        }
    }
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Exact(s) => text.contains(s),
            Self::Text(s) => text.to_lowercase().contains(s),
            Self::Regex(r) => r.find_iter(text).any(|found| !found.is_empty()),
        }
    }
    fn first_offset(&self, text: &str) -> Option<usize> {
        match self {
            Self::Exact(needle) => text.find(needle),
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
            Field::Text => {
                self.pattern.matches(text)
                    || self
                        .pattern
                        .matches(&path.file_name().unwrap_or_default().to_string_lossy())
            }
            Field::Content => self.pattern.matches(text),
            Field::File => self
                .pattern
                .matches(&path.file_name().unwrap_or_default().to_string_lossy()),
            Field::Path => self
                .pattern
                .matches(&path.to_string_lossy().replace('\\', "/")),
            Field::Tag => tags.iter().any(|tag| match &self.pattern {
                Pattern::Exact(s) => tag == s || tag.starts_with(&format!("{s}/")),
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
    expression: Option<group::Expression>,
}
pub struct LineMatch {
    pub offset: usize,
    pub line: usize,
    pub range: std::ops::Range<usize>,
    pub highlights: Vec<std::ops::Range<usize>>,
}
struct ScopedPattern<'a> {
    pattern: Option<&'a Pattern>,
    range: std::ops::Range<usize>,
}
impl ScopedPattern<'_> {
    fn first_offset(&self, text: &str) -> Option<usize> {
        self.pattern.map_or(Some(self.range.start), |pattern| {
            pattern
                .first_offset(&text[self.range.clone()])
                .map(|offset| offset + self.range.start)
        })
    }
    fn visit_ranges(&self, text: &str, mut visit: impl FnMut(std::ops::Range<usize>) -> bool) {
        if let Some(pattern) = self.pattern {
            pattern.visit_ranges(&text[self.range.clone()], |range| {
                visit(self.range.start + range.start..self.range.start + range.end)
            });
        } else {
            visit(self.range.start..self.range.start);
        }
    }
}

impl Query {
    pub fn parse(input: &str) -> Result<Self, String> {
        Self::parse_with_case(input, false)
    }
    pub fn parse_with_case(input: &str, case_sensitive: bool) -> Result<Self, String> {
        if let Some(expression) = group::parse(input, case_sensitive)? {
            return Ok(Self {
                groups: vec![],
                expression: Some(expression),
            });
        }
        Self::parse_flat(input, case_sensitive)
    }
    fn parse_flat(input: &str, case_sensitive: bool) -> Result<Self, String> {
        let mut words = vec![];
        let mut current = String::new();
        let mut quote = false;
        let mut regex = false;
        let mut escape = false;
        let (mut literal_prefix, mut quoted_value) = (false, false);
        let mut first_quote = None;
        for ch in input.chars() {
            if escape {
                if quote && !regex {
                    current.pop();
                }
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
                if !quote {
                    quoted_value = true;
                    first_quote.get_or_insert(current.len());
                    literal_prefix |= current.is_empty() || current == "-";
                }
                quote = !quote;
                continue;
            }
            if ch == '/'
                && !quote
                && (regex || current.is_empty() || current == "-" || current.ends_with(':'))
            {
                regex = !regex;
                current.push(ch);
                continue;
            }
            if ch.is_whitespace() && !quote && !regex {
                if !current.is_empty() {
                    words.push((
                        std::mem::take(&mut current),
                        literal_prefix,
                        quoted_value,
                        first_quote,
                    ));
                    literal_prefix = false;
                    quoted_value = false;
                    first_quote = None;
                }
            } else {
                current.push(ch);
            }
        }
        if quote || regex {
            return Err("搜索表达式中有未闭合的引号或正则表达式".into());
        }
        if !current.is_empty() {
            words.push((current, literal_prefix, quoted_value, first_quote));
        }
        let mut groups = vec![vec![]];
        let mut words = words.into_iter();
        while let Some((mut word, literal_prefix, mut quoted_value, mut first_quote)) = words.next()
        {
            if word == "OR" && !literal_prefix {
                if groups.last().is_some_and(Vec::is_empty) {
                    return Err("OR 前缺少搜索条件".into());
                }
                groups.push(vec![]);
                continue;
            }
            while !literal_prefix
                && !quoted_value
                && word.strip_suffix(':').is_some_and(|prefix| {
                    prefix.trim_start_matches('-').split(':').all(|part| {
                        matches!(
                            part.to_ascii_lowercase().as_str(),
                            "file" | "path" | "tag" | "content" | "match-case" | "ignore-case"
                        )
                    })
                })
            {
                let Some((next, quoted, value_quoted, next_quote)) = words.next() else {
                    return Err("搜索条件不能为空".into());
                };
                if next == "OR" && !quoted {
                    return Err("搜索条件不能为空".into());
                }
                if first_quote.is_none() {
                    first_quote = next_quote.map(|offset| word.len() + offset);
                }
                word.push_str(&next);
                quoted_value |= value_quoted;
            }
            let (exclude, word) = if first_quote == Some(0) {
                (false, word.as_str())
            } else {
                word.strip_prefix('-')
                    .map_or((false, word.as_str()), |s| (true, s))
            };
            let first_quote = first_quote.map(|offset| offset.saturating_sub(usize::from(exclude)));
            let (mut field, mut value, mut term_case_sensitive) =
                (Field::Text, word, case_sensitive);
            let mut field_set = false;
            if !literal_prefix {
                while let Some((prefix, rest)) = value.split_once(':') {
                    if first_quote
                        .is_some_and(|offset| word.len() - value.len() + prefix.len() + 1 > offset)
                    {
                        break;
                    }
                    match prefix.to_ascii_lowercase().as_str() {
                        "match-case" => term_case_sensitive = true,
                        "ignore-case" => term_case_sensitive = false,
                        "file" | "path" | "tag" | "content" => {
                            if field_set {
                                return Err("文件、路径、标签和正文条件不能互相嵌套。".into());
                            }
                            field_set = true;
                            field = match prefix.to_ascii_lowercase().as_str() {
                                "file" => Field::File,
                                "path" => Field::Path,
                                "tag" => Field::Tag,
                                _ => Field::Content,
                            };
                        }
                        _ => break,
                    }
                    value = rest;
                }
            }
            if matches!(field, Field::Tag) {
                value = value.trim_start_matches('#');
                term_case_sensitive = false;
            }
            if value.is_empty() {
                return Err("搜索条件不能为空".into());
            }
            let pattern = if !quoted_value
                && value.starts_with('/')
                && value.ends_with('/')
                && value.len() > 1
            {
                Pattern::Regex(
                    RegexBuilder::new(&value[1..value.len() - 1])
                        .case_insensitive(!term_case_sensitive)
                        .multi_line(true)
                        .size_limit(2 * 1024 * 1024)
                        .build()
                        .map_err(|e| format!("无效的正则表达式：{e}"))?,
                )
            } else if term_case_sensitive {
                Pattern::Exact(value.to_owned())
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
        Ok(Self {
            groups,
            expression: None,
        })
    }
    pub fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        if let Some(expression) = &self.expression {
            return expression.matches(path, text, tags);
        }
        self.groups.iter().any(|group| {
            !group.is_empty() && group.iter().all(|term| term.matches(path, text, tags))
        })
    }
    pub fn first_offset(&self, path: &Path, text: &str, tags: &[String]) -> Option<usize> {
        self.patterns(path, text, tags)
            .into_iter()
            .filter_map(|pattern| pattern.first_offset(text))
            .min()
    }
    pub fn title_highlights(
        &self,
        path: &Path,
        text: &str,
        tags: &[String],
    ) -> Vec<std::ops::Range<usize>> {
        let mut ranges = if let Some(expression) = &self.expression {
            expression.title_highlights(path, text, tags)
        } else {
            let label = path.to_string_lossy().replace('\\', "/");
            let name_start = label.rfind('/').map_or(0, |i| i + 1);
            let mut ranges = vec![];
            for group in self
                .groups
                .iter()
                .filter(|group| group.iter().all(|term| term.matches(path, text, tags)))
            {
                for term in group.iter().filter(|term| !term.exclude) {
                    let start = match term.field {
                        Field::Text | Field::File => name_start,
                        Field::Path => 0,
                        _ => continue,
                    };
                    term.pattern.visit_ranges(&label[start..], |range| {
                        ranges.push(start + range.start..start + range.end);
                        true
                    });
                }
            }
            ranges
        };
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<std::ops::Range<usize>> = vec![];
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && last.end >= range.start
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }
    fn patterns<'a>(&'a self, path: &Path, text: &str, tags: &[String]) -> Vec<ScopedPattern<'a>> {
        if let Some(expression) = &self.expression {
            return expression.patterns(path, text, tags);
        }
        self.groups
            .iter()
            .filter(|group| group.iter().all(|term| term.matches(path, text, tags)))
            .flat_map(|group| group.iter())
            .filter(|term| !term.exclude && matches!(term.field, Field::Text | Field::Content))
            .map(|term| ScopedPattern {
                pattern: Some(&term.pattern),
                range: 0..text.len(),
            })
            .collect()
    }
    /// Only terms from Boolean branches satisfied by the whole document may produce hits.
    pub fn matching_lines(
        &self,
        path: &Path,
        text: &str,
        tags: &[String],
        limit: usize,
    ) -> Vec<LineMatch> {
        let patterns = self.patterns(path, text, tags);
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
        let snippet = |line: usize, at: usize| {
            let start = starts[line];
            let end = starts.get(line + 1).copied().unwrap_or(text.len());
            let skip = text[start..at].chars().count().saturating_sub(40);
            let left = start
                + text[start..end]
                    .char_indices()
                    .nth(skip)
                    .map_or(end - start, |(i, _)| i);
            let right = left
                + text[left..end]
                    .char_indices()
                    .nth(120)
                    .map_or(end - left, |(i, _)| i);
            left..right
        };
        let mut matches = std::collections::BTreeMap::<usize, LineMatch>::new();
        for pattern in patterns {
            pattern.visit_ranges(text, |found| {
                let offset = found.start;
                let line = starts.partition_point(|start| *start <= offset) - 1;
                if matches.len() == limit
                    && matches
                        .last_key_value()
                        .is_some_and(|(last, _)| line > *last)
                {
                    return false;
                }
                let entry = matches.entry(line).or_insert_with(|| LineMatch {
                    offset,
                    line: line + 1,
                    range: snippet(line, offset),
                    highlights: vec![],
                });
                if offset < entry.offset {
                    entry.offset = offset;
                    entry.range = snippet(line, offset);
                    entry.highlights.retain_mut(|range| {
                        range.start = range.start.max(entry.range.start);
                        range.end = range.end.min(entry.range.end);
                        range.start < range.end
                    });
                }
                let mut range = found.start.max(entry.range.start)..found.end.min(entry.range.end);
                if range.start < range.end {
                    entry.highlights.retain(|old| {
                        if old.end < range.start || old.start > range.end {
                            true
                        } else {
                            range.start = range.start.min(old.start);
                            range.end = range.end.max(old.end);
                            false
                        }
                    });
                    entry.highlights.push(range);
                }
                if matches.len() > limit {
                    matches.pop_last();
                }
                true
            });
        }
        matches
            .into_values()
            .map(|mut found| {
                found.highlights.sort_by_key(|range| range.start);
                for range in &mut found.highlights {
                    range.start -= found.range.start;
                    range.end -= found.range.start;
                }
                found
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_queries_refresh_cached_ranges_when_completion_changes() {
        let query = Query::parse("task-todo:foo").unwrap();
        let path = Path::new("note.md");
        assert!(query.matches(path, "1. [ ] foo", &[]));
        assert!(!query.matches(path, "1. [X] foo", &[]));
        assert!(
            Query::parse("task-done:foo")
                .unwrap()
                .matches(path, "1. [>] foo", &[])
        );
        assert!(
            Query::parse("-task-todo:foo")
                .unwrap()
                .matches(path, "1. [X] foo", &[])
        );
        assert!(Query::parse("task:").is_err());
        assert!(Query::parse("file:task:\"\"").is_err());
        assert!(
            Query::parse("task-done:foo")
                .unwrap()
                .matches(path, "- [完] foo", &[])
        );
    }
    #[test]
    fn line_scope_accepts_single_terms_groups_and_case_prefixes() {
        let path = Path::new("Alpha.md");
        let query = Query::parse("match-case:line:(Alpha beta)").unwrap();
        assert!(query.matches(path, "Alpha beta", &[]));
        assert!(!query.matches(path, "alpha beta", &[]));
        assert!(!query.matches(path, "Alpha\nbeta", &[]));
        assert!(
            Query::parse("line: beta")
                .unwrap()
                .matches(path, "Alpha\nbeta", &[])
        );
        assert!(
            Query::parse("\"line:alpha\"")
                .unwrap()
                .matches(path, "line:alpha", &[])
        );
        assert!(Query::parse("line:").is_err());
        assert!(Query::parse("file:line:Alpha").is_err());
    }
    #[test]
    fn grouped_boolean_queries_support_precedence_negation_and_scoped_prefixes() {
        let path = Path::new("Note.md");
        let grouped = Query::parse("(alpha OR beta) project").unwrap();
        assert!(grouped.matches(path, "alpha project", &[]));
        assert!(grouped.matches(path, "beta project", &[]));
        assert!(!grouped.matches(path, "alpha", &[]));
        assert!(
            Query::parse("alpha OR (beta project)")
                .unwrap()
                .matches(path, "alpha", &[])
        );
        let excluded = Query::parse("-(draft OR archive) project").unwrap();
        assert!(excluded.matches(path, "project", &[]));
        assert!(!excluded.matches(path, "draft project", &[]));
        let scoped =
            Query::parse("file:(Note OR Other) match-case:(Alpha OR ignore-case:beta)").unwrap();
        assert!(scoped.matches(path, "Alpha", &[]));
        assert!(scoped.matches(path, "BETA", &[]));
        assert!(!scoped.matches(path, "alpha", &[]));
        assert!(
            Query::parse("(\"literal (value)\" OR /a(b|c)/)")
                .unwrap()
                .matches(path, "literal (value)", &[])
        );
        for invalid in ["(alpha", "alpha)", "()", "(alpha OR)", "OR (alpha)"] {
            assert!(Query::parse(invalid).is_err(), "{invalid}");
        }
        assert!(Query::parse(&format!("{}alpha{}", "(".repeat(70), ")".repeat(70))).is_err());
    }
    #[test]
    fn case_and_content_prefixes_respect_scope_spaces_and_quoted_literals() {
        let path = Path::new("Note.md");
        assert!(
            Query::parse("match-case:Alpha ignore-case:BETA")
                .unwrap()
                .matches(path, "Alpha beta", &[])
        );
        assert!(
            !Query::parse("match-case:alpha")
                .unwrap()
                .matches(path, "Alpha", &[])
        );
        assert!(
            Query::parse_with_case("ignore-case:alpha", true)
                .unwrap()
                .matches(path, "Alpha", &[])
        );
        assert!(
            Query::parse("match-case:file: Note")
                .unwrap()
                .matches(path, "", &[])
        );
        assert!(
            !Query::parse("content:Note")
                .unwrap()
                .matches(path, "body", &[])
        );
        assert!(
            Query::parse("content:/alpha beta/")
                .unwrap()
                .matches(path, "ALPHA BETA", &[])
        );
        for (query, text) in [
            ("\"OR\"", "OR"),
            ("\"file:missing\"", "file:missing"),
            ("content:\"/foo/\"", "/foo/"),
            ("match-case: \"file:\"", "file:"),
        ] {
            assert!(
                Query::parse(query).unwrap().matches(path, text, &[]),
                "{query}"
            );
        }
        assert!(
            !Query::parse("content:\"/foo/\"")
                .unwrap()
                .matches(path, "foo", &[])
        );
        assert!(Query::parse("match-case:").is_err());
        assert!(Query::parse("file:path:note").is_err());
        assert!(
            Query::parse("myfile: value")
                .unwrap()
                .matches(path, "myfile: other value", &[])
        );
        assert!(
            Query::parse(r#""say \"hi\"""#)
                .unwrap()
                .matches(path, "say \"hi\"", &[])
        );
        assert!(
            Query::parse("\"-blocked\"")
                .unwrap()
                .matches(path, "-blocked", &[])
        );
    }
    #[test]
    fn case_setting_affects_content_regex_and_paths_but_not_tags() {
        let path = Path::new("Folder/Note.md");
        let tags = vec!["Work/Task".into()];
        assert!(
            Query::parse("alpha file:note path:folder")
                .unwrap()
                .matches(path, "Alpha", &tags)
        );
        assert!(
            !Query::parse_with_case("alpha", true)
                .unwrap()
                .matches(path, "Alpha", &tags)
        );
        assert!(
            !Query::parse_with_case("file:note", true)
                .unwrap()
                .matches(path, "Alpha", &tags)
        );
        assert!(
            !Query::parse_with_case("path:folder", true)
                .unwrap()
                .matches(path, "Alpha", &tags)
        );
        assert!(
            Query::parse_with_case("tag:work", true)
                .unwrap()
                .matches(path, "Alpha", &tags)
        );
        let regex = Query::parse_with_case("/^Alpha/", true).unwrap();
        assert_eq!(
            regex.matching_lines(path, "Alpha\nalpha", &tags, 10).len(),
            1
        );
        assert_eq!(
            Query::parse_with_case("x", true)
                .unwrap()
                .first_offset(path, "İx", &tags),
            Some("İ".len())
        );
    }
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
