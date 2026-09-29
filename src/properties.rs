//! Small, source-preserving frontmatter editor. Unknown YAML stays verbatim.
use std::ops::Range;

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
        let values: Vec<_> = value
            .trim_matches(['[', ']'])
            .split(',')
            .map(|s| s.trim().trim_matches(['\'', '"']))
            .filter(|s| !s.is_empty())
            .collect();
        serde_json::to_string(&values).map_err(|e| e.to_string())?
    } else {
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(parsed) if !parsed.is_object() => parsed.to_string(),
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
