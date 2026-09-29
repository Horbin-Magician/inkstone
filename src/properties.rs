//! Small, source-preserving frontmatter editor. Unknown YAML stays verbatim.
use std::ops::Range;

/// Read reserved metadata through YAML, preserving commas, quoting and escapes in list items.
pub fn metadata(yaml: &str) -> (Vec<String>, Vec<String>) {
    use serde_json::Value;
    let Ok(Value::Object(values)) = serde_saphyr::from_str::<Value>(yaml) else {
        return (vec![], vec![]);
    };
    let strings = |name: &str, legacy: &str| -> Vec<String> {
        match values.get(name).or_else(|| values.get(legacy)) {
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
            && !tags.iter().any(|tag: &String| tag == value)
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

#[cfg(test)]
mod tests {
    use super::*;
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
