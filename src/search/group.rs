//! Parenthesized search expressions without expanding Boolean combinations.
use super::{Query, ScopedPattern};
use std::path::Path;

pub(super) enum Expression {
    Leaf(Box<Query>),
    All(Vec<Expression>),
    Any(Vec<Expression>),
    Not(Box<Expression>),
    Line(Box<Expression>),
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
            Self::Not(_) | Self::Line(_) => vec![],
        }
    }
    pub(super) fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        match self {
            Self::Leaf(query) => query.matches(path, text, tags),
            Self::All(items) => items.iter().all(|item| item.matches(path, text, tags)),
            Self::Any(items) => items.iter().any(|item| item.matches(path, text, tags)),
            Self::Not(item) => !item.matches(path, text, tags),
            Self::Line(item) => text.split('\n').any(|line| item.matches(path, line, tags)),
        }
    }
    pub(super) fn patterns<'a>(
        &'a self,
        path: &Path,
        text: &str,
        tags: &[String],
    ) -> Vec<ScopedPattern<'a>> {
        if !self.matches(path, text, tags) {
            return vec![];
        }
        match self {
            Self::Leaf(query) => query.patterns(path, text, tags),
            Self::All(items) | Self::Any(items) => items
                .iter()
                .flat_map(|item| item.patterns(path, text, tags))
                .collect(),
            Self::Not(_) => vec![],
            Self::Line(item) => {
                let mut result = vec![];
                let mut offset = 0;
                for line in text.split('\n') {
                    if item.matches(path, line, tags) {
                        let mut patterns = item.patterns(path, line, tags);
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
enum Token {
    Word(String),
    Open,
    Close,
    Or,
    Not,
}
fn tokens(input: &str) -> Vec<Token> {
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
        if ch == '/' && !quote && (regex || word.is_empty() || word == "-" || word.ends_with(':')) {
            regex = !regex;
            word.push(ch);
            continue;
        }
        if !quote && !regex {
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
fn line_parts(word: &str) -> Option<(&str, &str)> {
    let mut start = 0;
    while let Some((prefix, rest)) = word[start..].split_once(':') {
        match prefix.to_ascii_lowercase().as_str() {
            "line" => return Some((&word[..start], rest)),
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
                if let Some((prefix, rest)) = line_parts(word) {
                    let scope = format!("{scope}{prefix}content:");
                    let inner = if rest.is_empty() {
                        self.primary(&scope, depth + negatives + 1)?
                    } else {
                        let mut parser = Parser {
                            tokens: tokens(rest),
                            at: 0,
                            case_sensitive: self.case_sensitive,
                        };
                        parser.expression(&scope, depth + negatives + 1)?
                    };
                    let mut expression = Expression::Line(Box::new(inner));
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
                let value = format!("{scope}{word}");
                let mut expression = if prefix {
                    self.primary(&value, depth + negatives + 1)?
                } else {
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
        .any(|token| matches!(token, Token::Open | Token::Close) || matches!(token, Token::Word(word) if line_parts(word.trim_start_matches('-')).is_some()))
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
