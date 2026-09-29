//! Parenthesized search expressions without expanding Boolean combinations.
use super::{Pattern, Query};
use std::path::Path;

pub(super) enum Expression {
    Leaf(Box<Query>),
    All(Vec<Expression>),
    Any(Vec<Expression>),
    Not(Box<Expression>),
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
            Self::Not(_) => vec![],
        }
    }
    pub(super) fn matches(&self, path: &Path, text: &str, tags: &[String]) -> bool {
        match self {
            Self::Leaf(query) => query.matches(path, text, tags),
            Self::All(items) => items.iter().all(|item| item.matches(path, text, tags)),
            Self::Any(items) => items.iter().any(|item| item.matches(path, text, tags)),
            Self::Not(item) => !item.matches(path, text, tags),
        }
    }
    pub(super) fn patterns<'a>(
        &'a self,
        path: &Path,
        text: &str,
        tags: &[String],
    ) -> Vec<&'a Pattern> {
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
        .any(|token| matches!(token, Token::Open | Token::Close))
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
