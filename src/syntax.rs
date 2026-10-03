//! One source-coordinate parse shared by indexing and live presentation.
use markdown_parser::{ParseOptions, mdast::Node};
use std::sync::Arc;

pub fn options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

pub fn valid_inline_math(source: &str, range: &std::ops::Range<usize>) -> bool {
    let Some(raw) = source.get(range.clone()) else {
        return false;
    };
    if raw.starts_with("$$") {
        return true;
    }
    raw.starts_with('$')
        && raw.ends_with('$')
        && raw.len() > 2
        && raw[1..].chars().next().is_some_and(|c| !c.is_whitespace())
        && raw[..raw.len() - 1]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
        && !source[range.end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
}

pub fn parse_raw(source: &str) -> Option<Node> {
    fn definitions(node: &Node, source: &str, out: &mut String) {
        if matches!(node, Node::Definition(_) | Node::FootnoteDefinition(_))
            && let Some(p) = node.position()
        {
            out.push_str(&source[p.start.offset..p.end.offset]);
            out.push_str("\n\n");
            return;
        }
        if let Some(children) = node.children() {
            for n in children {
                definitions(n, source, out);
            }
        }
    }
    fn shift(node: &mut Node, origin: &markdown_parser::unist::Point) {
        if let Some(p) = node.position_mut() {
            for point in [&mut p.start, &mut p.end] {
                if point.line == 1 {
                    point.column += origin.column - 1;
                }
                point.line += origin.line - 1;
                point.offset += origin.offset;
            }
        }
        if let Some(children) = node.children_mut() {
            for n in children {
                shift(n, origin);
            }
        }
    }
    fn normalize(node: &mut Node, source: &str, defs: &str) {
        let Some(children) = node.children_mut() else {
            return;
        };
        let mut i = 0;
        while i < children.len() {
            let position = if matches!(&children[i], Node::InlineMath(_)) {
                children[i]
                    .position()
                    .cloned()
                    .filter(|p| !valid_inline_math(source, &(p.start.offset..p.end.offset)))
            } else {
                None
            };
            if let Some(p) = position {
                let mut prose = options();
                prose.constructs.math_text = false;
                prose.constructs.math_flow = false;
                let raw = &source[p.start.offset..p.end.offset];
                if let Ok(Node::Root(root)) =
                    markdown_parser::to_mdast(&format!("{raw}\n\n{defs}"), &prose)
                    && let Some(Node::Paragraph(paragraph)) = root.children.first()
                {
                    let mut replacement = paragraph.children.clone();
                    for n in &mut replacement {
                        shift(n, &p.start);
                    }
                    let len = replacement.len();
                    children.splice(i..=i, replacement);
                    i += len;
                    continue;
                }
            }
            normalize(&mut children[i], source, defs);
            i += 1;
        }
    }
    let mut ast = markdown_parser::to_mdast(source, &options()).ok()?;
    if !source.contains('$') {
        return Some(ast);
    }
    let mut defs = String::new();
    definitions(&ast, source, &mut defs);
    normalize(&mut ast, source, &defs);
    Some(ast)
}

#[derive(Clone)]
pub struct Snapshot {
    pub source: Arc<str>,
    pub structural: Arc<str>,
    pub comments: Vec<crate::comments::Comment>,
    pub ast: Option<Arc<Node>>,
    pub inline_footnotes: Vec<InlineFootnote>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InlineFootnote {
    pub range: std::ops::Range<usize>,
    pub content: std::ops::Range<usize>,
}

fn inline_footnotes(source: &str, ast: Option<&Node>) -> Vec<InlineFootnote> {
    fn excluded(node: &Node, out: &mut Vec<std::ops::Range<usize>>) {
        if matches!(
            node,
            Node::Code(_)
                | Node::InlineCode(_)
                | Node::Yaml(_)
                | Node::Html(_)
                | Node::Math(_)
                | Node::InlineMath(_)
                | Node::Definition(_)
                | Node::Link(_)
                | Node::Image(_)
        ) {
            if let Some(p) = node.position() {
                out.push(p.start.offset..p.end.offset);
            }
            return;
        }
        if let Some(children) = node.children() {
            for n in children {
                excluded(n, out);
            }
        }
    }
    let mut blocked = vec![];
    if let Some(ast) = ast {
        excluded(ast, &mut blocked);
    }
    blocked.sort_by_key(|r| r.start);
    let mut notes = vec![];
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find("^[") {
        let start = cursor + offset;
        cursor = start + 2;
        if source[..start]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count()
            % 2
            == 1
            || blocked.iter().any(|r| r.contains(&start))
        {
            continue;
        }
        let mut depth = 1;
        let mut i = cursor;
        while i < source.len() {
            if let Some(range) = blocked
                .get(blocked.partition_point(|r| r.end <= i))
                .filter(|r| r.contains(&i))
            {
                i = range.end;
                continue;
            }
            let ch = source[i..].chars().next().unwrap();
            if ch == '\\' {
                i += 1;
                if let Some(next) = source[i..].chars().next() {
                    i += next.len_utf8();
                }
                continue;
            }
            if ch == '[' {
                depth += 1;
            }
            if ch == ']' {
                depth -= 1;
                if depth == 0 {
                    if i > cursor {
                        notes.push(InlineFootnote {
                            range: start..i + 1,
                            content: cursor..i,
                        });
                    }
                    cursor = i + 1;
                    break;
                }
            }
            if ch == '\n' && source[i + 1..].trim_start_matches('\r').starts_with('\n') {
                break;
            }
            i += ch.len_utf8();
        }
    }
    notes
}

impl Snapshot {
    /// Reuse the tree for a plain, single-line paragraph edit. Markdown syntax,
    /// escaped text, comments and reference-like input always use the parser.
    /// This keeps synchronous live styles exact without reparsing long prose.
    pub fn update_plain_paragraph(&self, source: &str) -> Option<Self> {
        if !self.comments.is_empty() || !self.inline_footnotes.is_empty() {
            return None;
        }
        let before = self.source.as_ref();
        let start = before
            .chars()
            .zip(source.chars())
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        if start == before.len() && start == source.len() {
            return Some(self.clone());
        }
        let suffix = before[start..]
            .chars()
            .rev()
            .zip(source[start..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        let old_end = before.len() - suffix;
        let new_end = source.len() - suffix;
        let Node::Root(root) = self.ast.as_deref()? else {
            return None;
        };
        let (index, paragraph, text) = root.children.iter().enumerate().find_map(|(i, node)| {
            let Node::Paragraph(p) = node else {
                return None;
            };
            let [Node::Text(t)] = p.children.as_slice() else {
                return None;
            };
            let pos = p.position.as_ref()?;
            (pos.start.line == pos.end.line
                && pos.start.offset < start
                && old_end <= pos.end.offset
                && t.position.as_ref() == Some(pos)
                && before.get(pos.start.offset..pos.end.offset) == Some(t.value.as_str()))
            .then_some((i, p, t))
        })?;
        let position = paragraph.position.as_ref()?;
        let delta = new_end as isize - old_end as isize;
        let end = position.end.offset.checked_add_signed(delta)?;
        let value = source.get(position.start.offset..end)?;
        // These characters cannot introduce inline markup. GFM's bare www.
        // autolinks are the sole punctuation-based construct left in this set.
        if value.is_empty()
            || value.ends_with(' ')
            || value.to_ascii_lowercase().contains("www.")
            || !value.chars().all(|c| {
                c.is_alphanumeric()
                    || matches!(c, ' ' | '.' | ',')
                    || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
            })
        {
            return None;
        }
        // Changing a line prefix could turn prose into an ordered list.
        if !value.chars().next()?.is_alphabetic() {
            return None;
        }
        // markdown-rs columns, like offsets, count UTF-8 bytes (not scalars).
        let columns = delta;
        fn shift(node: &mut Node, at: usize, line: usize, bytes: isize, columns: isize) {
            if let Some(p) = node.position_mut() {
                for point in [&mut p.start, &mut p.end] {
                    if point.offset >= at {
                        point.offset = point.offset.checked_add_signed(bytes).unwrap();
                        if point.line == line {
                            point.column = point.column.checked_add_signed(columns).unwrap();
                        }
                    }
                }
            }
            if let Some(children) = node.children_mut() {
                for child in children {
                    shift(child, at, line, bytes, columns);
                }
            }
        }
        let mut ast = self.ast.as_deref()?.clone();
        shift(&mut ast, old_end, position.end.line, delta, columns);
        let Node::Paragraph(p) = &mut ast.children_mut()?[index] else {
            return None;
        };
        let Node::Text(t) = &mut p.children[0] else {
            return None;
        };
        debug_assert_eq!(
            text.value,
            before[position.start.offset..position.end.offset]
        );
        t.value = value.into();
        let source: Arc<str> = source.into();
        Some(Self {
            structural: source.clone(),
            source,
            ast: Some(Arc::new(ast)),
            comments: vec![],
            inline_footnotes: vec![],
        })
    }

    pub fn new(source: &str) -> Self {
        let comments = crate::comments::ranges(source);
        let structural: Arc<str> = crate::comments::masked(source, &comments).as_ref().into();
        let ast = parse_raw(&structural).map(Arc::new);
        let inline_footnotes = inline_footnotes(&structural, ast.as_deref());
        Self {
            source: source.into(),
            structural,
            comments,
            ast,
            inline_footnotes,
        }
    }
}

/// Attribute destinations in real HTML nodes. A small attribute lexer avoids
/// treating quoted titles, comments, or script strings as links.
pub fn html_destinations(ast: &Node, source: &str) -> Vec<(std::ops::Range<usize>, String)> {
    fn attributes(raw: &str, base: usize, out: &mut Vec<(std::ops::Range<usize>, String)>) {
        let lower = raw.to_ascii_lowercase();
        let bytes = raw.as_bytes();
        let mut cursor = 0;
        while let Some(relative) = raw[cursor..].find('<') {
            let open = cursor + relative;
            if raw[open..].starts_with("<!--") {
                cursor = raw[open..]
                    .find("-->")
                    .map_or(raw.len(), |end| open + end + 3);
                continue;
            }
            let mut i = open + 1;
            if bytes.get(i) == Some(&b'/') {
                i += 1;
            }
            let name_start = i;
            while bytes.get(i).is_some_and(u8::is_ascii_alphanumeric) {
                i += 1;
            }
            let tag = &lower[name_start..i];
            if matches!(tag, "script" | "style") {
                cursor = lower[i..]
                    .find(&format!("</{tag}"))
                    .map_or(raw.len(), |n| i + n + 2 + tag.len());
                continue;
            }
            while i < bytes.len() && bytes[i] != b'>' {
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                let start = i;
                while bytes
                    .get(i)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
                {
                    i += 1;
                }
                if start == i {
                    i += 1;
                    continue;
                }
                let attr = &lower[start..i];
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                if bytes.get(i) != Some(&b'=') {
                    continue;
                }
                i += 1;
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                let quote = bytes.get(i).copied().filter(|b| matches!(b, b'\'' | b'"'));
                if quote.is_some() {
                    i += 1;
                }
                let value_start = i;
                while bytes.get(i).is_some_and(|b| {
                    if let Some(q) = quote {
                        *b != q
                    } else {
                        !b.is_ascii_whitespace() && *b != b'>'
                    }
                }) {
                    i += 1;
                }
                if value_start < i
                    && ((tag == "a" && attr == "href") || (tag == "img" && attr == "src"))
                {
                    let value = &raw[value_start..i];
                    let parsed =
                        markdown_parser::to_mdast(&format!("[x](<{value}>)"), &options()).ok();
                    let decoded = parsed
                        .as_ref()
                        .and_then(Node::children)
                        .and_then(|n| n.first())
                        .and_then(Node::children)
                        .and_then(|n| n.first())
                        .and_then(|n| {
                            if let Node::Link(link) = n {
                                Some(link.url.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| value.into());
                    out.push((base + value_start..base + i, decoded));
                }
                if quote.is_some() && i < bytes.len() {
                    i += 1;
                }
            }
            cursor = (i + 1).min(raw.len());
        }
    }
    fn walk(node: &Node, source: &str, out: &mut Vec<(std::ops::Range<usize>, String)>) {
        if let Node::Html(html) = node
            && let Some(p) = &html.position
        {
            attributes(&source[p.start.offset..p.end.offset], p.start.offset, out);
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, out);
            }
        }
    }
    let mut out = vec![];
    walk(ast, source, &mut out);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn plain_paragraph_updates_match_full_parser_positions_and_content() {
        use super::Snapshot;
        for newline in ["\n", "\r\n"] {
            for paragraph in ["中文 English 😀 paragraph. x", "Alpha 中文 e\u{301} end"] {
                let source = format!(
                    "# Before{newline}{newline}{paragraph}{newline}{newline}# After{newline}\n**bold** [link](note.md)"
                );
                let snapshot = Snapshot::new(&source);
                let origin = source.find(paragraph).unwrap();
                for (at, ch) in paragraph.char_indices().skip(1) {
                    for replacement in ["", "中", "xyz", "👩‍💻", " e\u{301} "] {
                        let mut after = source.clone();
                        after.replace_range(origin + at..origin + at + ch.len_utf8(), replacement);
                        if let Some(updated) = snapshot.update_plain_paragraph(&after) {
                            let full = Snapshot::new(&after);
                            assert_eq!(updated.ast, full.ast, "{after:?}");
                            assert_eq!(
                                crate::markdown::spans_snapshot(&updated),
                                crate::markdown::spans_snapshot(&full)
                            );
                            assert_eq!(updated.structural, full.structural);
                        }
                    }
                }
                let after = source.replacen("English", "English中文😀", 1);
                assert!(snapshot.update_plain_paragraph(&after).is_some());
            }
        }
        let source = "# Heading\n\n中文 paragraph. x";
        let mut snapshot = Snapshot::new(source);
        let mut text = source.to_string();
        for suffix in ["y", "中", "😀", "a", " e\u{301}"] {
            text.push_str(suffix);
            snapshot = snapshot.update_plain_paragraph(&text).unwrap();
            assert_eq!(snapshot.ast, Snapshot::new(&text).ast);
        }
    }

    #[test]
    fn plain_paragraph_fast_path_rejects_structural_changes() {
        use super::Snapshot;
        for (before, after) in [
            ("Text end", "Text **end**"),
            ("Text end", "Text [[end]]"),
            ("Text end", "Text #end"),
            ("Text end", "Text www.example.com"),
            ("Text end", "Text WWW.example.com"),
            ("Text end", "Text\nend"),
            ("Text end", "Text end "),
            ("Text end", "1. Text end"),
            ("Text &amp; end", "Text &amp; ending"),
            ("Text %%comment%% end", "Text %%comment%% ending"),
            ("Text ^[note] end", "Text ^[note] ending"),
            ("> Text end", "> Text ending"),
            ("# Text end", "# Text ending"),
        ] {
            assert!(
                Snapshot::new(before)
                    .update_plain_paragraph(after)
                    .is_none(),
                "{after}"
            );
        }
    }

    use super::*;
    #[test]
    fn currency_remains_prose_with_real_links_comments_and_footnotes() {
        let source = "spent $5 [link][r] %%hide%% and $10; formula $x^2$ and [^n]\n\n[r]: target.md\n[^n]: body";
        let snapshot = Snapshot::new(source);
        let parsed = crate::index::parse_snapshot(&snapshot);
        assert_eq!(parsed.standard_links[0].0, "target.md");
        assert_eq!(parsed.comments.len(), 1);
        assert_eq!(parsed.footnote_references.len(), 1);
        let candidates = crate::preview::candidates(&snapshot);
        assert_eq!(candidates.len(), 1);
        assert_eq!(&source[candidates[0].source.clone()], "$x^2$");
    }
    #[test]
    fn inline_footnotes_keep_unicode_ranges_and_skip_escaped_code_and_math() {
        let source = r"正文^[短 **强调** [链接](note.md) \] 正文] `^[代码]` $\text{^[公式]}$ \^[转义] %%^[注释]%% ^[第二条]";
        let snapshot = Snapshot::new(source);
        assert_eq!(snapshot.inline_footnotes.len(), 2);
        let first = &snapshot.inline_footnotes[0];
        assert_eq!(
            &source[first.content.clone()],
            r"短 **强调** [链接](note.md) \] 正文"
        );
        assert_eq!(
            &source[first.range.clone()],
            r"^[短 **强调** [链接](note.md) \] 正文]"
        );
        assert_eq!(
            &source[snapshot.inline_footnotes[1].content.clone()],
            "第二条"
        );
        assert!(
            Snapshot::new("^[未闭合\n\n后续]")
                .inline_footnotes
                .is_empty()
        );
    }
    #[test]
    fn math_and_code_do_not_leak_note_structure_or_comment_markers() {
        let source = "# 真标题\r\n$\\text{[[伪链接]] #伪标签 %%公式%% ==文字==}$\r\n$$\r\n# 伪标题\r\n- [ ] 伪任务\r\n[[伪链接]]\r\n$$\r\n```mermaid\r\nA[\"[[伪链接]]\"]\r\n```\r\n[[真实]] %%注释%%";
        let snapshot = Snapshot::new(source);
        assert_eq!(snapshot.structural.len(), source.len());
        assert_eq!(snapshot.comments.len(), 1);
        let parsed = crate::index::parse_snapshot(&snapshot);
        assert_eq!(parsed.headings.len(), 1);
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].target, "真实");
        assert!(parsed.tasks.is_empty());
        assert!(parsed.tags.is_empty());
        let styles = crate::markdown::spans_snapshot(&snapshot);
        assert!(
            !styles
                .iter()
                .any(|s| s.kind == crate::markdown::Kind::Highlight)
        );
    }
}
