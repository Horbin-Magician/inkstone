//! Template body insertion and frontmatter merging as one source edit.
use crate::markdown_edit::Edit;
use serde_json::{Map, Value};
use std::ops::Range;

fn frontmatter(source: &str) -> Result<(usize, Map<String, Value>), String> {
    let Some(block) = crate::properties::block(source) else {
        if source.lines().next() == Some("---") {
            return Err("属性区缺少结束分隔线。".into());
        }
        return Ok((0, Map::new()));
    };
    let first = source.find('\n').unwrap() + 1;
    let close = source[..block.end]
        .trim_end_matches(['\r', '\n'])
        .rfind('\n')
        .unwrap()
        + 1;
    let yaml = &source[first..close];
    let value: Value =
        serde_saphyr::from_str(yaml).map_err(|error| format!("属性 YAML 无效：{error}"))?;
    match value {
        Value::Object(map) => Ok((block.end, map)),
        Value::Null => Ok((block.end, Map::new())),
        _ => Err("属性区必须是键值映射。".into()),
    }
}
fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
        && !matches!(value, Value::String(s) if s.is_empty())
        && !matches!(value, Value::Number(n) if n.as_f64() == Some(0.))
}
fn merge(existing: &mut Value, incoming: &Value) {
    if !truthy(existing) {
        *existing = incoming.clone();
        return;
    }
    match (existing, incoming) {
        (Value::Array(old), Value::Array(new)) => {
            for value in new {
                if !old.contains(value) {
                    old.push(value.clone());
                }
            }
        }
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in new {
                merge(old.entry(key).or_insert(Value::Null), value);
            }
        }
        (_, Value::Null) => (),
        (old, new) => *old = new.clone(),
    }
}
fn render(map: &Map<String, Value>, eol: &str) -> Result<String, String> {
    serde_saphyr::to_string(map)
        .map(|text| text.replace('\n', eol))
        .map_err(|error| error.to_string())
}
fn merge_header(
    source: &str,
    end: usize,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Result<String, String> {
    if before == after {
        return Ok(source[..end].to_string());
    }
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if end == 0 {
        return Ok(format!("---{eol}{}---{eol}", render(after, eol)?));
    }
    let properties = crate::properties::parse(source);
    // Patch ordinary top-level properties, leaving untouched YAML and comments verbatim.
    let simple =
        properties.len() == before.len() && properties.iter().all(|p| before.contains_key(&p.name));
    if !simple {
        return Err(
            "当前笔记包含复杂属性键或行内映射，暂不能自动合并；请在源码中合并属性。".into(),
        );
    }
    let mut result = source[..end].to_string();
    for property in properties.iter().rev() {
        if before.get(&property.name) != after.get(&property.name) {
            let mut single = Map::new();
            single.insert(property.name.clone(), after[&property.name].clone());
            result.replace_range(property.range.clone(), &render(&single, eol)?);
        }
    }
    let mut added = Map::new();
    for (key, value) in after {
        if !before.contains_key(key) {
            added.insert(key.clone(), value.clone());
        }
    }
    if !added.is_empty() {
        let close = result.trim_end_matches(['\r', '\n']).rfind('\n').unwrap() + 1;
        result.insert_str(close, &render(&added, eol)?);
    }
    Ok(result)
}
pub fn insert(source: &str, selection: Range<usize>, template: &str) -> Result<Edit, String> {
    if selection.start > selection.end
        || !source.is_char_boundary(selection.start)
        || !source.is_char_boundary(selection.end)
    {
        return Err("模板插入位置无效。".into());
    }
    // The reference inserts the complete template when the selection starts at document start.
    if selection.start == 0 {
        return Ok(Edit {
            range: selection,
            replacement: template.into(),
            selection: template.len()..template.len(),
        });
    }
    let (template_end, incoming) = frontmatter(template)?;
    if incoming.is_empty() {
        let caret = selection.start + template.len() - template_end;
        return Ok(Edit {
            range: selection,
            replacement: template[template_end..].into(),
            selection: caret..caret,
        });
    }
    let (source_end, existing) = frontmatter(source)?;
    if selection.start < source_end {
        return Err("请将光标移到属性区之后再插入模板。".into());
    }
    let mut merged = Value::Object(existing.clone());
    merge(&mut merged, &Value::Object(incoming));
    let header = merge_header(source, source_end, &existing, merged.as_object().unwrap())?;
    if frontmatter(&header)?.1 != *merged.as_object().unwrap() {
        return Err("属性合并会影响 YAML 引用，请在源码中合并。".into());
    }
    let mut replacement = header;
    replacement.push_str(&source[source_end..selection.start]);
    replacement.push_str(&template[template_end..]);
    let caret = replacement.len();
    // Keep the suffix outside the edit, and preserve its exact bytes.
    Ok(Edit {
        range: 0..selection.end,
        replacement,
        selection: caret..caret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merging_template_preserves_unrelated_yaml_and_body() {
        let source = "---\r\n# Keep\r\nunchanged: 'yes' # comment\r\ntags: [old]\r\nnested:\r\n  a: 1\r\n---\r\n中文😀结束";
        let at = source.find("😀").unwrap();
        let template = "---\ntags: [old, new]\nnested: {b: true}\nadded: 42\n---\n插入";
        let edit = insert(source, at..at, template).unwrap();
        let mut result = source.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert!(result.contains("# Keep\r\nunchanged: 'yes' # comment\r\n"));
        assert!(result.ends_with("中文插入😀结束"));
        let (_, map) = frontmatter(&result).unwrap();
        assert_eq!(map["tags"], serde_json::json!(["old", "new"]));
        assert_eq!(map["nested"], serde_json::json!({"a":1,"b":true}));
        assert_eq!(&result[edit.selection.start..], "😀结束");
    }
    #[test]
    fn template_start_invalid_yaml_and_null_rules() {
        let edit = insert("abc", 0..1, "---\ntitle: test\n---\nbody").unwrap();
        assert_eq!(edit.range, 0..1);
        assert!(edit.replacement.starts_with("---"));
        assert!(insert("abc", 1..1, "---\nbad: [\n---\nbody").is_err());
        let mut old = serde_json::json!({"keep":"value", "replace":false, "list":[1]});
        merge(
            &mut old,
            &serde_json::json!({"keep":null, "replace":null, "list":[1,2]}),
        );
        assert_eq!(
            old,
            serde_json::json!({"keep":"value","replace":null,"list":[1,2]})
        );
    }
    #[test]
    fn template_adds_properties_to_plain_and_empty_headers() {
        for source in ["正文", "---\n---\n正文", "---\n# 注释\n---\n正文"] {
            let at = source.len();
            let edit = insert(source, at..at, "---\ntitle: 测试\n---\n新增").unwrap();
            let mut result = source.to_string();
            result.replace_range(edit.range, &edit.replacement);
            assert_eq!(frontmatter(&result).unwrap().1["title"], "测试");
            assert!(result.ends_with("正文新增"));
            if source.contains("# 注释") {
                assert!(result.contains("# 注释"));
            }
        }
    }
}
