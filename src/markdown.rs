//! A deliberately bounded live-style scanner. It never rewrites source text.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading,
    Strong,
    Code,
    WikiLink,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub kind: Kind,
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub markers: Vec<Range<usize>>,
}
impl Span {
    pub fn active(&self, selection: &Range<usize>) -> bool {
        if selection.is_empty() {
            self.source.start <= selection.start && selection.start <= self.source.end
        } else {
            selection.start < self.source.end && selection.end > self.source.start
        }
    }
}

pub fn spans(text: &str) -> Vec<Span> {
    let mut result = Vec::new();
    let mut base = 0;
    let mut fence: Option<char> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let c = trimmed.chars().next().unwrap();
            if fence == Some(c) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(c);
            }
            base += line.len();
            continue;
        }
        if fence.is_some() || line.starts_with("    ") || line.starts_with('\t') {
            base += line.len();
            continue;
        }
        let body = line.trim_end_matches(['\r', '\n']);
        let hashes = body.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && body.as_bytes().get(hashes) == Some(&b' ') {
            result.push(Span {
                kind: Kind::Heading,
                source: base..base + body.len(),
                content: base + hashes + 1..base + body.len(),
                markers: vec![base..base + hashes + 1],
            });
        }
        let mut i = 0;
        while i < body.len() {
            let rest = &body[i..];
            if rest.starts_with('\\') {
                i += 1;
                if i < body.len() {
                    i += body[i..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            let pair = if rest.starts_with("[[") {
                Some(("[[", "]]", Kind::WikiLink))
            } else if rest.starts_with("**") {
                Some(("**", "**", Kind::Strong))
            } else if rest.starts_with('`') && !rest.starts_with("``") {
                Some(("`", "`", Kind::Code))
            } else {
                None
            };
            if let Some((open, close, kind)) = pair {
                let start = i + open.len();
                if let Some(relative) = body[start..].find(close) {
                    let end = start + relative;
                    if end > start {
                        let finish = end + close.len();
                        result.push(Span {
                            kind,
                            source: base + i..base + finish,
                            content: base + start..base + end,
                            markers: vec![base + i..base + start, base + end..base + finish],
                        });
                        i = finish;
                        continue;
                    }
                }
            }
            i += rest.chars().next().unwrap().len_utf8();
        }
        base += line.len();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_ranges_are_source_bytes_and_source_is_unchanged() {
        let s = "# 中文\r\n**粗体😀** 和 `代码` [[笔记]]\n";
        let items = spans(s);
        assert_eq!(items.len(), 4);
        assert_eq!(&s[items[1].content.clone()], "粗体😀");
        assert_eq!(&s[items[3].content.clone()], "笔记");
        for item in items {
            assert!(item.active(&(item.content.start..item.content.start)));
        }
    }
    #[test]
    fn code_fences_and_escaped_markers_stay_source() {
        assert!(spans("```md\n**不是粗体**\n```\n\\[[不是链接]]\n    **代码**").is_empty());
        assert_eq!(spans("`**literal**` [[真链接]]").len(), 2);
    }
}
