use super::{Pattern, Query, ScopedPattern};
use serde_json::Value;
use std::{cell::RefCell, path::Path};
mod value;
use value::Expected;
static HIGHLIGHT: std::sync::LazyLock<Pattern> =
    std::sync::LazyLock::new(|| Pattern::Regex(regex::Regex::new("[^\\r\\n]+").unwrap()));

enum Key {
    Exact(String),
    Query(Query),
}
pub(super) struct Property {
    key: Key,
    expected: Option<Expected>,
    cache: RefCell<Option<(String, Option<Value>)>>,
}
impl Property {
    pub(super) fn parse(input: &str, case_sensitive: bool) -> Result<Self, String> {
        let inner = input
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or("属性搜索方括号未闭合。")?;
        let (mut quote, mut regex, mut escape) = (false, false, false);
        let mut split = None;
        let mut previous = None;
        for (i, ch) in inner.char_indices() {
            if escape {
                escape = false;
                previous = Some(ch);
                continue;
            }
            if ch == '\\' {
                escape = true;
                previous = Some(ch);
                continue;
            }
            if ch == '"' && !regex {
                quote = !quote;
            }
            if ch == '/'
                && !quote
                && (regex
                    || previous.is_none_or(|p: char| p.is_whitespace() || matches!(p, ':' | '(')))
            {
                regex = !regex;
            }
            if !quote && !regex {
                if matches!(ch, '[' | ']') {
                    return Err("属性搜索不能嵌套方括号。".into());
                }
                if ch == ':' && split.is_none() {
                    split = Some(i);
                }
            }
            previous = Some(ch);
        }
        if quote || regex {
            return Err("属性搜索的引号或正则未闭合。".into());
        }
        let (name, value) = split.map_or((inner.trim(), None), |i| {
            (inner[..i].trim(), Some(inner[i + 1..].trim()))
        });
        if name.is_empty() {
            return Err("属性名条件不能为空。".into());
        }
        let key = if name.starts_with('"') && name.ends_with('"') {
            Key::Exact(serde_json::from_str::<String>(name).map_err(|_| "属性名引号格式无效。")?)
        } else {
            Key::Query(Query::parse_with_case(name, case_sensitive)?)
        };
        let expected = match value {
            None => None,
            Some("") => return Err("属性值条件不能为空，请使用 EMPTY 匹配空值。".into()),
            Some(value) => Some(Expected::parse(value, case_sensitive)?),
        };
        Ok(Self {
            key,
            expected,
            cache: RefCell::new(None),
        })
    }
    pub(super) fn matches(&self, source: &str) -> bool {
        !self.matching_keys(source).is_empty()
    }
    pub(super) fn patterns(&self, source: &str) -> Vec<ScopedPattern<'_>> {
        let keys = self.matching_keys(source);
        if keys.is_empty() {
            return vec![];
        }
        let Some(block) = crate::properties::block(source) else {
            return vec![];
        };
        let first = source.find('\n').unwrap() + 1;
        let last = source[..block.end]
            .trim_end_matches(['\r', '\n'])
            .rfind('\n')
            .unwrap()
            + 1;
        let Ok((_, _, entries)) = crate::yaml_source::entries(&source[first..last]) else {
            return vec![];
        };
        entries
            .into_iter()
            .filter(|entry| keys.contains(&entry.key))
            .map(|entry| ScopedPattern {
                pattern: Some(&HIGHLIGHT),
                range: first + entry.start..(first + entry.value.end).min(last),
            })
            .collect()
    }
    fn matching_keys(&self, source: &str) -> Vec<String> {
        let Some(block) = crate::properties::block(source) else {
            return vec![];
        };
        let header = &source[..block.end];
        if self
            .cache
            .borrow()
            .as_ref()
            .is_none_or(|(cached, _)| cached != header)
        {
            let first = header.find('\n').unwrap() + 1;
            let last = header.trim_end_matches(['\r', '\n']).rfind('\n').unwrap() + 1;
            let parsed = serde_saphyr::from_str::<Value>(&header[first..last]).ok();
            *self.cache.borrow_mut() = Some((header.into(), parsed));
        }
        let cache = self.cache.borrow();
        let Some((_, Some(Value::Object(values)))) = cache.as_ref() else {
            return vec![];
        };
        values
            .iter()
            .filter(|(name, value)| {
                let key_matches = match &self.key {
                    Key::Exact(key) => key.to_lowercase() == name.to_lowercase(),
                    Key::Query(query) => query.matches(Path::new(""), name, &[]),
                };
                if !key_matches {
                    return false;
                }
                let Some(expected) = &self.expected else {
                    return true;
                };
                match value {
                    Value::Array(items) => items.iter().any(|value| expected.matches(value)),
                    _ => expected.matches(value),
                }
            })
            .map(|(name, _)| name.clone())
            .collect()
    }
}
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(_) => "[object Object]".into(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    value_text(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => value.to_string(),
    }
}
