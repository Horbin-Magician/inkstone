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
struct YamlEntry {
    key: String,
    start: usize,
    value: Range<usize>,
}
fn entries(yaml: &str) -> Result<(bool, usize, Vec<YamlEntry>), String> {
    use granit_parser::{Event, Parser, StructureStyle};
    let mut depth = 0;
    let mut flow = false;
    let mut close = yaml.len();
    let mut result: Vec<YamlEntry> = vec![];
    let mut expect_key = true;
    let mut value_start = None;
    // Parser indexes are Unicode character offsets, not UTF-8 byte offsets.
    let offsets: Vec<_> = yaml
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(yaml.len()))
        .collect();
    for event in Parser::new_from_str(yaml) {
        let (event, span) = event.map_err(|e| e.to_string())?;
        let start = offsets[span.start.index()];
        let end = offsets[span.end.index()];
        match event {
            Event::Comment(..)
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(..)
            | Event::DocumentEnd => continue,
            Event::MappingStart(style, ..) if depth == 0 => {
                flow = style == StructureStyle::Flow;
                depth = 1;
            }
            Event::Scalar(key, ..) if depth == 1 && expect_key => {
                result.push(YamlEntry {
                    key: key.into_owned(),
                    start,
                    value: end..end,
                });
                expect_key = false;
            }
            Event::MappingStart(..) | Event::SequenceStart(..) => {
                if depth == 1 {
                    if expect_key {
                        return Err("暂不支持复合 YAML 属性键。".into());
                    }
                    value_start = Some(start);
                }
                depth += 1;
            }
            Event::MappingEnd | Event::SequenceEnd => {
                if depth == 1 {
                    close = start;
                }
                depth -= 1;
                if depth == 1 {
                    result.last_mut().unwrap().value = value_start.take().unwrap()..end;
                    expect_key = true;
                }
            }
            Event::Scalar(..) | Event::Alias(..) if depth == 1 => {
                result.last_mut().ok_or("属性键无效。")?.value = start..end;
                expect_key = true;
            }
            _ => (),
        }
    }
    Ok((flow, close, result))
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
    let first = source.find('\n').unwrap() + 1;
    let closing = source[..end]
        .trim_end_matches(['\r', '\n'])
        .rfind('\n')
        .unwrap()
        + 1;
    let yaml = &source[first..closing];
    let (flow, close, properties) = entries(yaml)?;
    let simple =
        properties.len() == before.len() && properties.iter().all(|p| before.contains_key(&p.key));
    if !simple {
        return Err("当前笔记包含 YAML 合并键，暂不能自动合并；请在源码中合并属性。".into());
    }
    let mut result = source[..end].to_string();
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    for (i, property) in properties.iter().enumerate() {
        if before.get(&property.key) != after.get(&property.key) {
            if flow {
                edits.push((
                    first + property.value.start..first + property.value.end,
                    serde_json::to_string(&after[&property.key]).map_err(|e| e.to_string())?,
                ));
                continue;
            }
            let mut single = Map::new();
            single.insert(property.key.clone(), after[&property.key].clone());
            let mut stop = properties.get(i + 1).map(|p| p.start).unwrap_or(yaml.len());
            // Keep standalone comments belonging to the next property outside this edit.
            for line in yaml[property.start..stop].split_inclusive('\n').rev() {
                if line.trim().is_empty() || line.starts_with('#') {
                    stop -= line.len();
                } else {
                    break;
                }
            }
            edits.push((first + property.start..first + stop, render(&single, eol)?));
        }
    }
    let mut added = Map::new();
    for (key, value) in after {
        if !before.contains_key(key) {
            added.insert(key.clone(), value.clone());
        }
    }
    if !added.is_empty() {
        if flow {
            let json = serde_json::to_string(&added).map_err(|e| e.to_string())?;
            let prefix = if before.is_empty() || yaml[..close].trim_end().ends_with(',') {
                ""
            } else {
                ", "
            };
            edits.push((
                first + close..first + close,
                format!("{prefix}{}", &json[1..json.len() - 1]),
            ));
        } else {
            edits.push((closing..closing, render(&added, eol)?));
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    for (range, replacement) in edits.into_iter().rev() {
        result.replace_range(range, &replacement);
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
    fn quoted_keys_keep_unrelated_comments_and_multiline_values() {
        let source = "---\n\"名称: 中文\": old\n# 保留下一属性说明\n'unchanged key': |\n  a: b\n  中文😀\n---\n正文";
        let template = "---\n\"名称: 中文\": new\n---\n插入";
        let edit = insert(source, source.len()..source.len(), template).unwrap();
        let mut result = source.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert_eq!(frontmatter(&result).unwrap().1["名称: 中文"], "new");
        assert!(result.contains("# 保留下一属性说明\n'unchanged key': |\n  a: b\n  中文😀\n"));
        assert!(result.ends_with("正文插入"));
    }
    #[test]
    fn flow_frontmatter_merges_values_and_adds_keys_without_reformatting_neighbors() {
        for yaml in [
            "{\"名称: 中文\": old, keep: '原样', nested: {a: 1}, tags: [a]}",
            "{\n  \"名称: 中文\": old, # 保留注释\n  keep: '原样', nested: {a: 1}, tags: [a],\n}",
        ] {
            let source = format!("---\r\n{yaml}\r\n---\r\n正文");
            let template =
                "---\n\"名称: 中文\": new\nnested: {b: 2}\ntags: [b]\nadded: true\n---\n插入";
            let edit = insert(&source, source.len()..source.len(), template).unwrap();
            let mut result = source.clone();
            result.replace_range(edit.range, &edit.replacement);
            let properties = frontmatter(&result).unwrap().1;
            assert_eq!(properties["名称: 中文"], "new");
            assert_eq!(properties["nested"], serde_json::json!({"a":1,"b":2}));
            assert_eq!(properties["tags"], serde_json::json!(["a", "b"]));
            assert_eq!(properties["added"], true);
            assert!(result.contains("keep: '原样'"));
            if yaml.contains('#') {
                assert!(result.contains("# 保留注释"));
            }
            assert!(result.ends_with("正文插入"));
        }
    }
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
