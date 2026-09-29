//! Source positions of top-level YAML entries.
use std::ops::Range;

pub(crate) struct YamlEntry {
    pub key: String,
    pub start: usize,
    pub value: Range<usize>,
}
pub(crate) fn entries(yaml: &str) -> Result<(bool, usize, Vec<YamlEntry>), String> {
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
