use crate::search::{
    Query,
    group::{Token, tokens},
};
use serde_json::Value;
use std::path::Path;

pub(super) enum Expected {
    Query(Query),
    True,
    False,
    Empty,
    All(Vec<Expected>),
    Any(Vec<Expected>),
    Not(Box<Expected>),
    Compare(bool, String),
}
impl Expected {
    pub(super) fn parse(input: &str, case_sensitive: bool) -> Result<Self, String> {
        let mut parser = Parser {
            tokens: tokens(input),
            at: 0,
            case_sensitive,
        };
        let value = parser.expression(0)?;
        if parser.at != parser.tokens.len() {
            return Err("属性值括号位置无效。".into());
        }
        Ok(value)
    }
    pub(super) fn matches(&self, value: &Value) -> bool {
        match self {
            Self::True => value == &Value::Bool(true),
            Self::False => value == &Value::Bool(false),
            Self::Empty => value.is_null(),
            Self::Query(query) => query.matches(Path::new(""), &super::value_text(value), &[]),
            Self::All(items) => items.iter().all(|item| item.matches(value)),
            Self::Any(items) => items.iter().any(|item| item.matches(value)),
            Self::Not(item) => !item.matches(value),
            Self::Compare(less, expected) => {
                let ordering = match value {
                    Value::String(value) => Some(value.cmp(expected)),
                    Value::Number(value) => value
                        .as_f64()
                        .zip(expected.parse::<f64>().ok())
                        .and_then(|(a, b)| a.partial_cmp(&b)),
                    Value::Bool(value) => expected
                        .parse::<f64>()
                        .ok()
                        .and_then(|b| (u8::from(*value) as f64).partial_cmp(&b)),
                    _ => None,
                };
                ordering
                    == Some(if *less {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    })
            }
        }
    }
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    case_sensitive: bool,
}
impl Parser {
    fn expression(&mut self, depth: usize) -> Result<Expected, String> {
        let mut branches = vec![self.conjunction(depth)?];
        while self.tokens.get(self.at) == Some(&Token::Or) {
            self.at += 1;
            branches.push(self.conjunction(depth)?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Expected::Any(branches)
        })
    }
    fn conjunction(&mut self, depth: usize) -> Result<Expected, String> {
        let mut items = vec![];
        while !matches!(
            self.tokens.get(self.at),
            None | Some(Token::Close | Token::Or)
        ) {
            items.push(self.primary(depth)?);
        }
        if items.is_empty() {
            return Err("属性值条件不能为空。".into());
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Expected::All(items)
        })
    }
    fn primary(&mut self, depth: usize) -> Result<Expected, String> {
        if depth > 64 {
            return Err("属性条件嵌套过深。".into());
        }
        let token = self.tokens.get(self.at).ok_or("属性值条件不完整。")?;
        self.at += 1;
        match token {
            Token::Open => {
                let result = self.expression(depth + 1)?;
                if self.tokens.get(self.at) != Some(&Token::Close) {
                    return Err("属性值括号未闭合。".into());
                }
                self.at += 1;
                Ok(result)
            }
            Token::Not => Ok(Expected::Not(Box::new(self.primary(depth + 1)?))),
            Token::Word(word) => {
                let negative = word.chars().take_while(|ch| *ch == '-').count();
                if depth + negative > 64 {
                    return Err("属性条件嵌套过深。".into());
                }
                let word = &word[negative..];
                let mut result = match word {
                    "TRUE" => Expected::True,
                    "FALSE" => Expected::False,
                    "EMPTY" => Expected::Empty,
                    _ if word.starts_with(['>', '<']) => {
                        let less = word.starts_with('<');
                        let operand = if word.len() > 1 {
                            word[1..].to_owned()
                        } else {
                            let Some(Token::Word(value)) = self.tokens.get(self.at) else {
                                return Err("比较条件缺少值。".into());
                            };
                            self.at += 1;
                            value.clone()
                        };
                        let operand = serde_json::from_str::<String>(&operand).unwrap_or(operand);
                        Expected::Compare(less, operand)
                    }
                    _ => Expected::Query(Query::parse_with_case(word, self.case_sensitive)?),
                };
                for _ in 0..negative {
                    result = Expected::Not(Box::new(result));
                }
                Ok(result)
            }
            _ => Err("属性值条件无效。".into()),
        }
    }
}
