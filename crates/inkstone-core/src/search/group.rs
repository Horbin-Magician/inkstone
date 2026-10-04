//! Parenthesized search expressions without expanding Boolean combinations.
use super::{Query, ScopedPattern};
use std::path::Path;

pub(super) enum Expression {
    Always,
    Leaf(Box<Query>),
    All(Vec<Expression>),
    Any(Vec<Expression>),
    Not(Box<Expression>),
    Line(Box<Expression>),
    Region(super::regions::Regions, Box<Expression>),
    Section(super::sections::Sections, Box<Expression>),
    Property(Box<super::property::Property>),
}
impl Expression {
    pub(super) fn title_highlights(
        &self,
        path: &Path,
        text: &str,
        tags: &[String],
    ) -> Vec<std::ops::Range<usize>> {
        if !self.matches(path, text, tags) {
            return vec![];
        }
        match self {
            Self::Leaf(query) => query.title_highlights(path, text, tags),
            Self::All(items) | Self::Any(items) => items
                .iter()
                .flat_map(|item| item.title_highlights(path, text, tags))
                .collect(),
            Self::Always
            | Self::Not(_)
            | Self::Line(_)
            | Self::Region(..)
            | Self::Section(..)
            | Self::Property(_) => {
                vec![]
            }
        }
    }
    pub(super) fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        self.matches_in(path, text, tags, None, text)
    }
    fn matches_in<'t>(
        &self,
        path: &Path,
        text: &'t str,
        tags: &[String],
        context: Option<&super::sections::Context<'t>>,
        original: &'t str,
    ) -> bool {
        match self {
            Self::Always => true,
            Self::Property(property) => property.matches(original),
            Self::Leaf(query) => query.matches(path, text, tags),
            Self::All(items) => items
                .iter()
                .all(|item| item.matches_in(path, text, tags, context, original)),
            Self::Any(items) => items
                .iter()
                .any(|item| item.matches_in(path, text, tags, context, original)),
            Self::Not(item) => !item.matches_in(path, text, tags, context, original),
            Self::Section(cache, item) => {
                let view = super::sections::View::new(cache, text, context);
                view.candidates.clone().any(|i| {
                    item.matches_in(
                        path,
                        &view.source[view.sections[i].range.clone()],
                        tags,
                        Some(&view.context(i)),
                        original,
                    )
                })
            }
            Self::Line(item) => text
                .split('\n')
                .any(|line| item.matches_in(path, line, tags, None, original)),
            Self::Region(regions, item) => regions
                .ranges(text)
                .iter()
                .any(|range| item.matches_in(path, &text[range.clone()], tags, None, original)),
        }
    }
    pub(super) fn patterns<'a>(
        &'a self,
        path: &Path,
        text: &str,
        tags: &[String],
    ) -> Vec<ScopedPattern<'a>> {
        self.patterns_in(path, text, tags, None, text)
    }
    fn patterns_in<'a, 't>(
        &'a self,
        path: &Path,
        text: &'t str,
        tags: &[String],
        context: Option<&super::sections::Context<'t>>,
        original: &'t str,
    ) -> Vec<ScopedPattern<'a>> {
        if !self.matches_in(path, text, tags, context, original) {
            return vec![];
        }
        match self {
            Self::Leaf(query) => query.patterns(path, text, tags),
            Self::All(items) | Self::Any(items) => items
                .iter()
                .flat_map(|item| item.patterns_in(path, text, tags, context, original))
                .collect(),
            Self::Always | Self::Not(_) => vec![],
            Self::Property(property) => {
                if text.as_ptr() == original.as_ptr() && text.len() == original.len() {
                    property.patterns(original)
                } else {
                    vec![]
                }
            }
            Self::Section(cache, item) => {
                let view = super::sections::View::new(cache, text, context);
                let mut result = vec![];
                for i in view.candidates.clone() {
                    let section = &view.sections[i];
                    let body = &view.source[section.range.clone()];
                    let context = view.context(i);
                    if item.matches_in(path, body, tags, Some(&context), original) {
                        let mut patterns =
                            item.patterns_in(path, body, tags, Some(&context), original);
                        if patterns.is_empty() {
                            patterns.push(ScopedPattern {
                                pattern: None,
                                range: 0..body.len(),
                            });
                        }
                        for pattern in &mut patterns {
                            pattern.range.start += section.range.start - view.base;
                            pattern.range.end += section.range.start - view.base;
                        }
                        result.extend(patterns);
                    }
                }
                result
            }
            Self::Region(regions, item) => {
                let mut result = vec![];
                for range in regions.ranges(text).iter() {
                    let body = &text[range.clone()];
                    if item.matches_in(path, body, tags, None, original) {
                        let mut patterns = item.patterns_in(path, body, tags, None, original);
                        if patterns.is_empty() {
                            patterns.push(ScopedPattern {
                                pattern: None,
                                range: 0..body.len(),
                            });
                        }
                        for pattern in &mut patterns {
                            pattern.range.start += range.start;
                            pattern.range.end += range.start;
                        }
                        result.extend(patterns);
                    }
                }
                result
            }
            Self::Line(item) => {
                let mut result = vec![];
                let mut offset = 0;
                for line in text.split('\n') {
                    if item.matches_in(path, line, tags, None, original) {
                        let mut patterns = item.patterns_in(path, line, tags, None, original);
                        if patterns.is_empty() {
                            patterns.push(ScopedPattern {
                                pattern: None,
                                range: 0..line.len(),
                            });
                        }
                        for pattern in &mut patterns {
                            pattern.range.start += offset;
                            pattern.range.end += offset;
                        }
                        result.extend(patterns);
                    }
                    offset += line.len() + 1;
                }
                result
            }
        }
    }
}
#[derive(PartialEq)]
pub(super) enum Token {
    Word(String),
    Open,
    Close,
    Or,
    Not,
}
pub(super) fn tokens(input: &str) -> Vec<Token> {
    fn flush(word: &mut String, out: &mut Vec<Token>) {
        if word.is_empty() {
            return;
        }
        let word = std::mem::take(word);
        if word == "OR" {
            out.push(Token::Or);
        } else if word.chars().all(|ch| ch == '-') {
            out.extend(word.chars().map(|_| Token::Not));
        } else {
            out.push(Token::Word(word));
        }
    }
    let mut result = vec![];
    let mut word = String::new();
    let (mut quote, mut regex, mut escape) = (false, false, false);
    let mut brackets = 0usize;
    for ch in input.chars() {
        if escape {
            word.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            word.push(ch);
            escape = true;
            continue;
        }
        if ch == '"' && !regex {
            quote = !quote;
            word.push(ch);
            continue;
        }
        if ch == '/'
            && !quote
            && (regex
                || word.is_empty()
                || word == "-"
                || word.ends_with(':')
                || (brackets > 0
                    && word
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_whitespace() || matches!(c, '[' | '('))))
        {
            regex = !regex;
            word.push(ch);
            continue;
        }
        if !quote && !regex {
            if ch == '[' {
                brackets += 1;
                word.push(ch);
                continue;
            }
            if ch == ']' && brackets > 0 {
                brackets -= 1;
                word.push(ch);
                if brackets == 0 {
                    flush(&mut word, &mut result);
                }
                continue;
            }
            if brackets > 0 {
                word.push(ch);
                continue;
            }
            if ch.is_whitespace() {
                flush(&mut word, &mut result);
                continue;
            }
            if ch == '(' || ch == ')' {
                flush(&mut word, &mut result);
                result.push(if ch == '(' { Token::Open } else { Token::Close });
                continue;
            }
        }
        word.push(ch);
    }
    flush(&mut word, &mut result);
    result
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    case_sensitive: bool,
}
fn property_parts(word: &str) -> Option<(&str, &str)> {
    let mut start = 0;
    loop {
        if word[start..].starts_with('[') {
            return Some((&word[..start], &word[start..]));
        }
        let (prefix, _) = word[start..].split_once(':')?;
        if !matches!(
            prefix.to_ascii_lowercase().as_str(),
            "match-case" | "ignore-case"
        ) {
            return None;
        }
        start += prefix.len() + 1;
    }
}
enum Scope {
    Line,
    Task(Option<bool>),
    Block,
    Section,
}
fn normalized_scope(scope: &str) -> String {
    let mut result = String::new();
    let mut section = false;
    for part in scope.split_terminator(':') {
        if part.eq_ignore_ascii_case("section") {
            if !section {
                result.push_str("content:");
                section = true;
            }
        } else {
            result.push_str(part);
            result.push(':');
        }
    }
    result
}
fn scope_parts(word: &str) -> Option<(&str, &str, Scope)> {
    let mut start = 0;
    while let Some((prefix, rest)) = word[start..].split_once(':') {
        match prefix.to_ascii_lowercase().as_str() {
            "line" => return Some((&word[..start], rest, Scope::Line)),
            "block" => return Some((&word[..start], rest, Scope::Block)),
            "section" => return Some((&word[..start], rest, Scope::Section)),
            "task" => return Some((&word[..start], rest, Scope::Task(None))),
            "task-todo" => return Some((&word[..start], rest, Scope::Task(Some(false)))),
            "task-done" => return Some((&word[..start], rest, Scope::Task(Some(true)))),
            "match-case" | "ignore-case" | "file" | "path" | "content" | "tag" => {
                start += prefix.len() + 1
            }
            _ => break,
        }
    }
    None
}
impl Parser {
    fn expression(&mut self, scope: &str, depth: usize) -> Result<Expression, String> {
        let mut branches = vec![self.conjunction(scope, depth)?];
        while self.tokens.get(self.at) == Some(&Token::Or) {
            self.at += 1;
            branches.push(self.conjunction(scope, depth)?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Expression::Any(branches)
        })
    }
    fn conjunction(&mut self, scope: &str, depth: usize) -> Result<Expression, String> {
        let mut items = vec![];
        while !matches!(
            self.tokens.get(self.at),
            None | Some(Token::Close | Token::Or)
        ) {
            items.push(self.primary(scope, depth)?);
        }
        if items.is_empty() {
            return Err("括号或 OR 缺少搜索条件。".into());
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Expression::All(items)
        })
    }
    fn primary(&mut self, scope: &str, depth: usize) -> Result<Expression, String> {
        if depth > 64 {
            return Err("搜索表达式嵌套过深。".into());
        }
        let token = self.tokens.get(self.at).ok_or("搜索条件不完整。")?;
        self.at += 1;
        match token {
            Token::Not => Ok(Expression::Not(Box::new(self.primary(scope, depth + 1)?))),
            Token::Open => {
                let expression = self.expression(scope, depth + 1)?;
                if self.tokens.get(self.at) != Some(&Token::Close) {
                    return Err("搜索括号未闭合。".into());
                }
                self.at += 1;
                Ok(expression)
            }
            Token::Word(word) => {
                let negatives = word.chars().take_while(|ch| *ch == '-').count();
                if depth + negatives > 64 {
                    return Err("搜索表达式嵌套过深。".into());
                }
                let word = &word[negatives..];
                if let Some((prefix, property)) = property_parts(word) {
                    let mut case_sensitive = self.case_sensitive;
                    for part in format!("{scope}{prefix}").split_terminator(':') {
                        match part.to_ascii_lowercase().as_str() {
                            "match-case" => case_sensitive = true,
                            "ignore-case" => case_sensitive = false,
                            _ => (),
                        }
                    }
                    let mut expression = Expression::Property(Box::new(
                        super::property::Property::parse(property, case_sensitive)?,
                    ));
                    for _ in 0..negatives {
                        expression = Expression::Not(Box::new(expression));
                    }
                    return Ok(expression);
                }
                if let Some((prefix, rest, kind)) = scope_parts(word) {
                    let scope = format!(
                        "{scope}{prefix}{}",
                        if matches!(kind, Scope::Section) {
                            "section:"
                        } else {
                            "content:"
                        }
                    );
                    Query::parse_flat(
                        &format!("{}scope-check", normalized_scope(&scope)),
                        self.case_sensitive,
                    )?;
                    let all_tasks = matches!(kind, Scope::Task(_))
                        && (rest == "\"\""
                            || (rest.is_empty()
                                && matches!(self.tokens.get(self.at), Some(Token::Word(value)) if value == "\"\"")));
                    let inner = if all_tasks {
                        if rest.is_empty() {
                            self.at += 1;
                        }
                        Expression::Always
                    } else if rest.is_empty() {
                        self.primary(&scope, depth + negatives + 1)?
                    } else {
                        let mut parser = Parser {
                            tokens: tokens(rest),
                            at: 0,
                            case_sensitive: self.case_sensitive,
                        };
                        parser.expression(&scope, depth + negatives + 1)?
                    };
                    let mut expression = match kind {
                        Scope::Line => Expression::Line(Box::new(inner)),
                        Scope::Task(completed) => Expression::Region(
                            super::regions::Regions::new(super::regions::Kind::Task(completed)),
                            Box::new(inner),
                        ),
                        Scope::Block => Expression::Region(
                            super::regions::Regions::new(super::regions::Kind::Block),
                            Box::new(inner),
                        ),
                        Scope::Section => Expression::Section(Default::default(), Box::new(inner)),
                    };
                    for _ in 0..negatives {
                        expression = Expression::Not(Box::new(expression));
                    }
                    return Ok(expression);
                }
                let prefix = word.strip_suffix(':').is_some_and(|prefix| {
                    prefix.split(':').all(|part| {
                        matches!(
                            part.to_ascii_lowercase().as_str(),
                            "file" | "path" | "content" | "tag" | "match-case" | "ignore-case"
                        )
                    })
                });
                let mut expression = if prefix {
                    self.primary(&format!("{scope}{word}"), depth + negatives + 1)?
                } else {
                    let value = format!("{}{word}", normalized_scope(scope));
                    Expression::Leaf(Box::new(Query::parse_flat(&value, self.case_sensitive)?))
                };
                for _ in 0..negatives {
                    expression = Expression::Not(Box::new(expression));
                }
                Ok(expression)
            }
            _ => Err("搜索括号或 OR 位置无效。".into()),
        }
    }
}
pub(super) fn parse(input: &str, case_sensitive: bool) -> Result<Option<Expression>, String> {
    let tokens = tokens(input);
    if !tokens
        .iter()
        .any(|token| matches!(token, Token::Open | Token::Close) || matches!(token, Token::Word(word) if scope_parts(word.trim_start_matches('-')).is_some() || property_parts(word.trim_start_matches('-')).is_some()))
    {
        return Ok(None);
    }
    let mut parser = Parser {
        tokens,
        at: 0,
        case_sensitive,
    };
    let root = parser.expression("", 0)?;
    if parser.at != parser.tokens.len() {
        return Err("搜索表达式包含多余的右括号。".into());
    }
    Ok(Some(root))
}
