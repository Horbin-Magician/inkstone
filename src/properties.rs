//! Small, source-preserving frontmatter editor. Unknown YAML stays verbatim.
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    #[default]
    Text,
    List,
    Number,
    Checkbox,
    Date,
    DateTime,
    Other,
}
impl Kind {
    pub fn editor_value(self, value: &str) -> String {
        let parsed = serde_json::from_str::<serde_json::Value>(value);
        match (self, parsed) {
            (Self::List, Ok(serde_json::Value::String(text))) => {
                serde_json::to_string(&[text]).unwrap()
            }
            (Self::List, Ok(serde_json::Value::Null)) => "[]".into(),
            (_, Ok(serde_json::Value::String(text))) => text,
            (_, Ok(serde_json::Value::Null)) => String::new(),
            _ => value.to_owned(),
        }
    }
    pub fn infer(value: &str) -> Self {
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(serde_json::Value::Array(_)) => Self::List,
            Ok(serde_json::Value::Number(_)) => Self::Number,
            Ok(serde_json::Value::Bool(_)) => Self::Checkbox,
            Ok(serde_json::Value::Object(_)) => Self::Other,
            _ => Self::Text,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::List => "列表",
            Self::Number => "数字",
            Self::Checkbox => "复选框",
            Self::Date => "日期",
            Self::DateTime => "日期与时间",
            Self::Other => "其他",
        }
    }
    pub fn encode(self, input: &str) -> Result<String, String> {
        use serde_json::Value;
        let trimmed = input.trim();
        match self {
            Self::Text => serde_json::to_string(input).map_err(|e| e.to_string()),
            Self::List => Ok(parse(&set("", "aliases", input)?)[0].value.clone()),
            _ if trimmed.is_empty() => Ok("null".into()),
            Self::Number => match serde_json::from_str::<Value>(trimmed) {
                Ok(value @ Value::Number(_)) => Ok(value.to_string()),
                _ => Err("请输入有效数字。".into()),
            },
            Self::Checkbox => match trimmed {
                "true" | "false" => Ok(trimmed.into()),
                _ => Err("复选框值必须是 true 或 false。".into()),
            },
            Self::Date => {
                let date = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                    .map_err(|_| "请使用 YYYY-MM-DD 日期格式。")?;
                if date.format("%Y-%m-%d").to_string() != trimmed {
                    return Err("请使用 YYYY-MM-DD 日期格式。".into());
                }
                Ok(serde_json::to_string(trimmed).unwrap())
            }
            Self::DateTime => {
                if chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M").is_err()
                    && chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").is_err()
                {
                    return Err("请使用 YYYY-MM-DDTHH:mm 日期时间格式。".into());
                }
                Ok(serde_json::to_string(trimmed).unwrap())
            }
            Self::Other => serde_json::from_str::<Value>(input)
                .map(|v| v.to_string())
                .map_err(|_| "复杂属性请使用有效 JSON 或在源码中编辑。".into()),
        }
    }
}

/// Read reserved metadata through YAML, preserving commas, quoting and escapes in list items.
pub fn metadata(yaml: &str) -> (Vec<String>, Vec<String>) {
    use serde_json::Value;
    let Ok(Value::Object(values)) = serde_saphyr::from_str::<Value>(yaml) else {
        return (vec![], vec![]);
    };
    let strings = |name: &str, legacy: &str| -> Vec<String> {
        let find = |key: &str| {
            values.get(key).or_else(|| {
                values
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value)
            })
        };
        match find(name).or_else(|| find(legacy)) {
            Some(Value::String(value)) => vec![value.clone()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => vec![],
        }
    };
    let mut tags = vec![];
    for value in strings("tags", "tag") {
        let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
        if !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
            && value.chars().any(|c| !c.is_numeric())
        {
            tags.push(value.to_owned());
        }
    }
    let mut aliases = vec![];
    for value in strings("aliases", "alias") {
        let value = value.trim();
        if !value.is_empty()
            && !value.contains(['\n', '\r'])
            && !aliases.iter().any(|alias: &String| alias == value)
        {
            aliases.push(value.to_owned());
        }
    }
    (tags, aliases)
}

#[derive(Clone, Debug)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub range: Range<usize>,
}
pub fn block(source: &str) -> Option<Range<usize>> {
    let first = source.split_inclusive('\n').next()?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }
    let mut offset = first.len();
    for line in source[offset..].split_inclusive('\n') {
        if matches!(line.trim_end_matches(['\r', '\n']), "---" | "...") {
            return Some(0..offset + line.len());
        }
        offset += line.len();
    }
    None
}
pub fn parse(source: &str) -> Vec<Property> {
    let Some(block) = block(source) else {
        return vec![];
    };
    let first = source.split_inclusive('\n').next().unwrap().len();
    let mut offset = first;
    let mut properties: Vec<Property> = vec![];
    for line in source[first..block.end].split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if matches!(trimmed, "---" | "...") {
            break;
        }
        if !line.starts_with([' ', '\t', '#'])
            && let Some((name, value)) = trimmed.split_once(':')
        {
            properties.push(Property {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
                range: offset..offset + line.len(),
            });
        } else if line.starts_with([' ', '\t', '-'])
            && let Some(previous) = properties.last_mut()
        {
            previous.range.end = offset + line.len();
        }
        offset += line.len();
    }
    for property in &mut properties {
        if let Ok(serde_json::Value::Object(values)) =
            serde_saphyr::from_str::<serde_json::Value>(&source[property.range.clone()])
            && let Some(value) = values.get(&property.name)
        {
            property.value = value.to_string();
        }
    }
    properties
}
pub fn set(source: &str, key: &str, value: &str) -> Result<String, String> {
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err("属性名只能包含文字、数字、下划线或连字符".into());
    }
    let value = if matches!(key, "tags" | "aliases") {
        let input = value.trim();
        let yaml = if input.starts_with('[') || input.starts_with("- ") {
            input.to_owned()
        } else {
            format!("[{input}]")
        };
        let parsed: serde_json::Value = serde_saphyr::from_str(&yaml)
            .map_err(|_| "列表格式无效；包含逗号、冒号等字符的项目请用引号包围。".to_string())?;
        let items = parsed.as_array().ok_or("请使用文本列表。")?;
        let values = items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or("标签和别名必须是文本；数字等内容请用引号包围。")
            })
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_string(&values).map_err(|e| e.to_string())?
    } else {
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(parsed) => parsed.to_string(),
            _ => serde_json::to_string(value).map_err(|e| e.to_string())?,
        }
    };
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let line = format!("{key}: {value}{eol}");
    let mut result = source.to_string();
    if let Some(property) = parse(source).into_iter().find(|p| p.name == key) {
        result.replace_range(property.range, &line);
    } else if block(source).is_some() {
        let first = source.find('\n').unwrap() + 1;
        result.insert_str(first, &line);
    } else {
        result = format!("---{eol}{line}---{eol}{source}");
    }
    Ok(result)
}

fn header_value(source: &str) -> Result<serde_json::Value, String> {
    let block = block(source).ok_or("属性已不存在，请重新打开属性编辑。")?;
    let first = source.find('\n').unwrap() + 1;
    let close = source[..block.end]
        .trim_end_matches(['\r', '\n'])
        .rfind('\n')
        .unwrap()
        + 1;
    serde_saphyr::from_str(&source[first..close])
        .map_err(|_| "属性 YAML 无效，请先在源码中修正。".into())
}
fn checked_property(source: &str, key: &str) -> Result<Property, String> {
    let values = header_value(source)?;
    if !values.as_object().is_some_and(|map| map.contains_key(key)) {
        return Err("属性不存在或使用了暂不支持的复杂键。".into());
    }
    parse(source)
        .into_iter()
        .find(|p| p.name == key)
        .ok_or_else(|| "属性位置无法识别。".into())
}
fn validate_header(source: &str) -> Result<(), String> {
    if block(source).is_some() {
        header_value(source).map_err(|_| "修改会破坏 YAML 引用，请先在源码中处理。")?;
    }
    Ok(())
}
pub fn edit(
    source: &str,
    original: Option<&str>,
    key: &str,
    value: &str,
) -> Result<String, String> {
    let Some(original) = original else {
        if block(source).is_some()
            && header_value(source)?
                .as_object()
                .is_some_and(|map| map.contains_key(key))
        {
            return Err("同名属性已存在，请编辑已有属性。".into());
        }
        return set(source, key, value);
    };
    let property = checked_property(source, original)?;
    if original != key
        && header_value(source)?
            .as_object()
            .is_some_and(|map| map.contains_key(key))
    {
        return Err("同名属性已存在，请使用其他名称。".into());
    }
    let rendered = set("", key, value)?;
    let line = parse(&rendered).into_iter().next().ok_or("属性值无效。")?;
    let mut replacement = rendered[line.range].to_owned();
    if source.contains("\r\n") {
        replacement = replacement.replace('\n', "\r\n");
    }
    let mut result = source.to_owned();
    result.replace_range(property.range, &replacement);
    validate_header(&result)?;
    Ok(result)
}
pub fn remove(source: &str, key: &str) -> Result<String, String> {
    let property = checked_property(source, key)?;
    let mut result = source.to_owned();
    result.replace_range(property.range, "");
    validate_header(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_list_values_remain_single_items_in_the_editor() {
        assert_eq!(
            Kind::List.editor_value("\"Smith, John\""),
            "[\"Smith, John\"]"
        );
        assert_eq!(Kind::List.editor_value("null"), "[]");
        assert_eq!(Kind::Text.editor_value("\"Smith, John\""), "Smith, John");
        assert_eq!(
            Kind::List.editor_value("[\"one\",\"two\"]"),
            "[\"one\",\"two\"]"
        );
    }
    #[test]
    fn explicit_property_types_preserve_text_and_validate_values() {
        assert_eq!(Kind::infer("\"123\""), Kind::Text);
        assert_eq!(Kind::Text.encode("123").unwrap(), "\"123\"");
        assert_eq!(Kind::Text.encode("true").unwrap(), "\"true\"");
        assert_eq!(Kind::Number.encode("12.5").unwrap(), "12.5");
        assert!(Kind::Number.encode("NaN").is_err());
        assert!(Kind::Checkbox.encode("yes").is_err());
        assert_eq!(Kind::Checkbox.encode("false").unwrap(), "false");
        assert!(Kind::Date.encode("2025-02-29").is_err());
        assert!(Kind::Date.encode("2024-2-29").is_err());
        assert_eq!(Kind::Date.encode("2024-02-29").unwrap(), "\"2024-02-29\"");
        assert!(Kind::DateTime.encode("2026-09-29T25:30").is_err());
        assert!(Kind::DateTime.encode("2026-09-29T12:30").is_ok());
    }
    #[test]
    fn rename_and_remove_properties_preserve_neighbors_and_reject_collisions() {
        let source = "---\r\n# keep\r\nold: [one, two]\r\nother: value\r\n---\r\n正文😀";
        let renamed = edit(source, Some("old"), "new", "[\"one\",\"two\"]").unwrap();
        assert!(!renamed.contains("old:"));
        assert!(renamed.contains("# keep\r\nnew: [\"one\",\"two\"]\r\nother: value\r\n"));
        assert!(edit(source, Some("old"), "other", "x").is_err());
        assert!(edit(source, Some("missing"), "new", "x").is_err());
        let removed = remove(source, "old").unwrap();
        assert_eq!(removed, "---\r\n# keep\r\nother: value\r\n---\r\n正文😀");
        assert!(remove("---\nanchor: &a value\nref: *a\n---\n", "anchor").is_err());
    }
    #[test]
    fn list_properties_roundtrip_through_editor_without_splitting_items() {
        let source = "---\r\naliases:\r\n  - 'Smith, John'\r\n  - 'It''s a note'\r\n  - 名称\r\nother: '原文' # keep\r\n---\r\n正文😀";
        let property = parse(source)
            .into_iter()
            .find(|p| p.name == "aliases")
            .unwrap();
        let updated = set(source, "aliases", &property.value).unwrap();
        assert_eq!(
            crate::index::parse(&updated).aliases,
            ["Smith, John", "It's a note", "名称"]
        );
        assert!(updated.contains("other: '原文' # keep\r\n"));
        assert!(updated.ends_with("正文😀"));
        assert_eq!(
            parse("---\ntitle: 'It''s a note'\n---\n")[0].value,
            "\"It's a note\""
        );
        assert_eq!(
            crate::index::parse(&set("正文", "aliases", "'Smith, John', \"名称: 冒号\"").unwrap())
                .aliases,
            ["Smith, John", "名称: 冒号"]
        );
        assert!(set(source, "aliases", "[broken").is_err());
        assert!(set(source, "aliases", "[true, 42]").is_err());
        assert!(
            crate::index::parse(&set(source, "aliases", "").unwrap())
                .aliases
                .is_empty()
        );
    }
    #[test]
    fn reserved_metadata_uses_yaml_values_without_splitting_aliases() {
        let (tags, aliases) = metadata(
            "aliases: [\"Smith, John\", 'It''s a note', \"\\u4e2d文\", '#名称', 'Smith, John']\ntags: ['#工作/项目', valid-tag, 123, '123', true, {nested: bad}]\n",
        );
        assert_eq!(aliases, ["Smith, John", "It's a note", "中文", "#名称"]);
        assert_eq!(tags, ["工作/项目", "valid-tag"]);
        assert_eq!(
            metadata(
                "aliases:\n  - '名称: 带冒号' # comment\n  - \"引号\\\"测试\"\ntags:\n  - 标签\n"
            )
            .1,
            ["名称: 带冒号", "引号\"测试"]
        );
        assert_eq!(
            metadata("alias: 单个别名\ntag: old-tag\n"),
            (vec!["old-tag".into()], vec!["单个别名".into()])
        );
        assert!(metadata("aliases: []\nalias: ignored\n").1.is_empty());
        assert_eq!(metadata("aliases: [broken\ntags: [bad]"), (vec![], vec![]));
    }
    #[test]
    fn property_changes_preserve_body_and_unrelated_yaml() {
        let source = "---\r\ntitle: old\r\nunknown:\r\n  nested: yes\r\n---\r\n# 原文😀\r\n";
        let result = set(source, "title", "新标题").unwrap();
        assert!(result.contains("unknown:\r\n  nested: yes\r\n"));
        assert!(result.ends_with("# 原文😀\r\n"));
        assert!(result.contains("title: \"新标题\"\r\n"));
        let result = set(&result, "tags", "工作, 中文").unwrap();
        assert!(result.contains("tags: [\"工作\",\"中文\"]\r\n"));
        assert!(set(source, "bad\nkey", "x").is_err());
    }
}
