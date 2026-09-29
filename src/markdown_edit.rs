//! Source edits for Markdown list and quote keystrokes.
use std::ops::Range;
mod indentation;
mod renumber;

/// A minimal indentation change for one line, with offsets relative to that line.
pub fn indentation_change(
    line: &str,
    width: usize,
    tabs: bool,
    outdent: bool,
) -> Option<(Range<usize>, String)> {
    indentation::line_change(line, width.clamp(2, 8), tabs, outdent)
}

#[derive(Clone, Copy)]
pub enum Key {
    Enter,
    SoftEnter,
    Indent,
    Outdent,
    Backspace,
}

#[derive(Debug)]
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}
/// Insert at the start of the selection without consuming its text, as in Obsidian.
/// The reference and trailing definition form one undoable source edit.
pub fn insert_footnote(text: &str, selection: Range<usize>) -> Option<Edit> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let parsed = crate::index::parse(text);
    let mut largest = "0";
    for (_, id) in &parsed.footnote_definitions {
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            let normalized = id.trim_start_matches('0');
            if normalized.len() > largest.len()
                || (normalized.len() == largest.len() && normalized > largest)
            {
                largest = normalized;
            }
        }
    }
    // Decimal increment also supports identifiers larger than machine integers.
    let mut digits = largest.as_bytes().to_vec();
    let mut carry = true;
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            carry = false;
            break;
        }
    }
    if carry {
        digits.insert(0, b'1');
    }
    let id = String::from_utf8(digits).ok()?;
    let marker = format!("[^{id}]");
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let tail = &text[selection.end..];
    let trailing = tail
        .chars()
        .rev()
        .take_while(|c| *c == '\n' || *c == '\r')
        .filter(|c| *c == '\n')
        .count()
        .min(2);
    let replacement = format!(
        "{marker}{}{}{marker}: {newline}",
        &text[selection.start..],
        newline.repeat(2 - trailing)
    );
    let cursor = selection.start + marker.len();
    Some(Edit {
        range: selection.start..text.len(),
        replacement,
        selection: cursor..cursor,
    })
}

pub fn insert_link(text: &str, selection: Range<usize>, wiki: bool) -> Option<Edit> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let label = &text[selection.clone()];
    let (replacement, cursor) = if wiki {
        (format!("[[{label}]]"), selection.start + 2 + label.len())
    } else {
        (
            format!("[{label}]()"),
            selection.start
                + if selection.is_empty() {
                    1
                } else {
                    label.len() + 3
                },
        )
    };
    Some(Edit {
        range: selection,
        replacement,
        selection: cursor..cursor,
    })
}
#[derive(Clone, Copy)]
pub enum BlockFormat {
    Heading(u8),
    Quote,
    Bullet,
    Numbered,
    Code,
    Callout,
}

pub fn block_format(text: &str, selection: Range<usize>, format: BlockFormat) -> Option<Edit> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let cursor = if !selection.is_empty() && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let mut end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    if end > start && text.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    let raw = &text[start..end];
    if matches!(format, BlockFormat::Callout) {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let prefix = format!("> [!note]{newline}> ");
        let map = |pos: usize| {
            let at = pos.min(end);
            start + prefix.len() + at - start
                + text[start..at].bytes().filter(|b| *b == b'\n').count() * 2
        };
        return Some(Edit {
            range: start..end,
            replacement: format!("{prefix}{}", raw.replace('\n', "\n> ")),
            selection: map(selection.start)..map(selection.end),
        });
    }
    if matches!(format, BlockFormat::Code) {
        let ticks = raw
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1;
        let fence = "`".repeat(ticks);
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let prefix = format!("{fence}{newline}");
        return Some(Edit {
            range: start..end,
            replacement: format!("{prefix}{raw}{newline}{fence}"),
            selection: selection.start.min(end) + prefix.len()
                ..selection.end.min(end) + prefix.len(),
        });
    }
    static PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?P<base>(?:[ \t]*>[ \t]?)*[ \t]*)(?P<head>#{1,6}(?:[ \t]+|$))?(?P<list>(?:[-+*]|[0-9]{1,9}[.)])[ \t]+)?").unwrap()
    });
    let rows: Vec<_> = if raw.is_empty() {
        vec![""]
    } else {
        raw.split_inclusive('\n').collect()
    };
    let mut changes: Vec<(Range<usize>, String, bool)> = vec![];
    let mut offset = start;
    let mut counters = std::collections::BTreeMap::<String, usize>::new();
    for row in rows {
        let content = row.trim_end_matches(['\r', '\n']);
        if content.trim().is_empty() && !raw.is_empty() {
            offset += row.len();
            continue;
        }
        let captures = PREFIX.captures(content)?;
        let base = captures.name("base")?;
        let (range, replacement, already) = match format {
            BlockFormat::Heading(level) => {
                let level = level.min(6);
                let old = captures.name("head");
                let marker = if level == 0 {
                    String::new()
                } else {
                    format!("{} ", "#".repeat(level as usize))
                };
                let already = old.is_some_and(|m| m.as_str().trim().len() == level as usize);
                (
                    offset + base.end()..offset + old.map_or(base.end(), |m| m.end()),
                    marker,
                    already,
                )
            }
            BlockFormat::Quote => {
                let indent = content
                    .bytes()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count();
                let tail = &content[indent..];
                let width = if tail.starts_with("> ") {
                    2
                } else if tail.starts_with('>') {
                    1
                } else {
                    0
                };
                (
                    offset + indent..offset + indent + width,
                    "> ".into(),
                    width > 0,
                )
            }
            BlockFormat::Bullet | BlockFormat::Numbered => {
                let head = captures.name("head");
                let old = if head.is_some() {
                    None
                } else {
                    captures.name("list")
                };
                let at = old.map_or(base.end(), |m| m.start());
                let n = counters.entry(base.as_str().into()).or_default();
                *n += 1;
                let numbered = matches!(format, BlockFormat::Numbered);
                let already =
                    old.is_some_and(|m| m.as_str().starts_with(char::is_numeric) == numbered);
                (
                    offset + at
                        ..offset + old.map_or_else(|| head.map_or(at, |m| m.end()), |m| m.end()),
                    if numbered {
                        format!("{n}. ")
                    } else {
                        "- ".into()
                    },
                    already,
                )
            }
            BlockFormat::Code | BlockFormat::Callout => unreachable!(),
        };
        if !content.trim().is_empty() || raw.is_empty() {
            changes.push((range, replacement, already));
        }
        offset += row.len();
    }
    let remove = !matches!(format, BlockFormat::Heading(_))
        && !changes.is_empty()
        && changes.iter().all(|(_, _, yes)| *yes);
    let mut result = raw.to_string();
    let mut mapped = selection.clone();
    let mut delta = 0isize;
    for (range, replacement, _) in &mut changes {
        if remove {
            replacement.clear();
        }
        let map = |pos: usize| {
            if pos < range.start {
                pos
            } else if pos >= range.end {
                pos.saturating_add_signed(replacement.len() as isize - range.len() as isize)
            } else {
                range.start + replacement.len()
            }
        };
        if selection.start >= range.start {
            mapped.start = map(selection.start).saturating_add_signed(delta);
        }
        if selection.end >= range.start {
            mapped.end = map(selection.end).saturating_add_signed(delta);
        }
        delta += replacement.len() as isize - range.len() as isize;
    }
    for (range, replacement, _) in changes.into_iter().rev() {
        result.replace_range(range.start - start..range.end - start, &replacement);
    }
    Some(Edit {
        range: start..end,
        replacement: result,
        selection: mapped,
    })
}

/// Toggle an inline format using parsed ranges, so escaped markers and nested
/// emphasis are not mistaken for the requested style.
pub fn inline_format(text: &str, selection: Range<usize>, left: &str, right: &str) -> Option<Edit> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let kind = match left {
        "**" => Some(crate::markdown::Kind::Strong),
        "*" => Some(crate::markdown::Kind::Emphasis),
        "~~" => Some(crate::markdown::Kind::Strike),
        "==" => Some(crate::markdown::Kind::Highlight),
        "`" => Some(crate::markdown::Kind::Code),
        _ => None,
    };
    let spans = if kind.is_some() {
        crate::markdown::spans(text)
    } else {
        vec![]
    };
    if let Some(kind) = kind
        && let Some(span) = spans.iter().find(|s| {
            if s.kind != kind {
                return false;
            }
            let mut inner = s.content.clone();
            while let Some(nested) = spans
                .iter()
                .find(|n| n.source == inner && n.content != inner)
            {
                inner = nested.content.clone();
            }
            s.source == selection
                || s.content == selection
                || (!selection.is_empty() && inner == selection)
        })
    {
        let replacement = text[span.content.clone()].to_string();
        let after = selection
            .start
            .max(span.content.start)
            .min(span.content.end)
            - span.content.start
            + span.source.start
            ..selection.end.max(span.content.start).min(span.content.end) - span.content.start
                + span.source.start;
        return Some(Edit {
            selection: after,
            range: span.source.clone(),
            replacement,
        });
    }
    if selection.is_empty()
        && left == right
        && selection.start >= left.len()
        && text.get(selection.start - left.len()..selection.start) == Some(left)
        && text.get(selection.end..selection.end + right.len()) == Some(right)
    {
        let start = selection.start - left.len();
        if text[..start]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count()
            % 2
            == 0
        {
            return Some(Edit {
                range: start..selection.end + right.len(),
                replacement: String::new(),
                selection: start..start,
            });
        }
    }
    let selected = &text[selection.clone()];
    let (left, right) = if left == "`" {
        let run = selected
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            + 1;
        let marker = "`".repeat(run);
        let padding = if selected.starts_with('`') || selected.ends_with('`') {
            " "
        } else {
            ""
        };
        (format!("{marker}{padding}"), format!("{padding}{marker}"))
    } else {
        (left.to_string(), right.to_string())
    };
    let selected_after = selection.start + left.len()..selection.end + left.len();
    Some(Edit {
        range: selection,
        replacement: format!("{left}{selected}{right}"),
        selection: selected_after,
    })
}

struct Prefix<'a> {
    quote: &'a str,
    indent: &'a str,
    marker: &'a str,
    gap: &'a str,
    task: bool,
    end: usize,
}

fn prefix(line: &str) -> Option<Prefix<'_>> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?P<quote>(?:[ \t]*>[ \t]?)*)?(?P<indent>[ \t]*)(?:(?P<marker>[-+*]|[0-9]{1,9}[.)])(?P<gap>[ \t]+))?(?P<task>\[[ xX]\](?:[ \t]+|$))?").unwrap()
    });
    let c = RE.captures(line)?;
    let get = |name| c.name(name).map_or("", |m| m.as_str());
    let marker = get("marker");
    let quote = get("quote");
    if marker.is_empty() && quote.is_empty() {
        return None;
    }
    // A checkbox without a list marker is literal quote text.
    let end = if marker.is_empty() {
        quote.len() + get("indent").len()
    } else {
        c.get(0)?.end()
    };
    Some(Prefix {
        quote,
        indent: get("indent"),
        marker,
        gap: get("gap"),
        task: !marker.is_empty() && c.name("task").is_some(),
        end,
    })
}

fn literal_ranges(text: &str) -> Vec<Range<usize>> {
    use markdown_parser::mdast::Node;
    fn walk(node: &Node, ranges: &mut Vec<Range<usize>>) {
        if matches!(
            node,
            Node::Code(_)
                | Node::Yaml(_)
                | Node::Toml(_)
                | Node::Html(_)
                | Node::Math(_)
                | Node::ThematicBreak(_)
        ) {
            if let Some(p) = node.position() {
                ranges.push(p.start.offset..p.end.offset);
            }
            return;
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, ranges);
            }
        }
    }
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let mut ranges = vec![];
    if let Ok(root) = markdown_parser::to_mdast(text, &options) {
        walk(&root, &mut ranges);
    }
    ranges
}

fn continuation_prefix<'a>(text: &'a str, start: usize, line: &'a str) -> Option<Prefix<'a>> {
    let direct = prefix(line);
    if direct.as_ref().is_some_and(|p| !p.marker.is_empty()) {
        return direct;
    }
    let width = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | '>'))
        .map_or(line.len(), |(i, _)| i);
    if width > 0 {
        for previous in text[..start].lines().rev() {
            if let Some(p) = prefix(previous) {
                if p.end == width && !p.marker.is_empty() {
                    return Some(p);
                }
                if p.end < width {
                    break;
                }
            } else {
                let indent = previous
                    .bytes()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count();
                if indent < width {
                    break;
                }
            }
        }
    }
    direct
}

fn outdent(indent: &str, width: usize) -> &str {
    if let Some(rest) = indent.strip_prefix('\t') {
        rest
    } else {
        &indent[indent
            .bytes()
            .take_while(|b| *b == b' ')
            .take(width)
            .count()..]
    }
}

/// Return `None` to let the ordinary editor handle the key.
pub fn edit(text: &str, selection: Range<usize>, key: Key) -> Option<Edit> {
    edit_with_indent(text, selection, key, 4, false)
}

pub fn edit_with_indent(
    text: &str,
    selection: Range<usize>,
    key: Key,
    width: usize,
    use_tabs: bool,
) -> Option<Edit> {
    edit_with_options(text, selection, key, width, use_tabs, true)
}

pub fn edit_with_options(
    text: &str,
    selection: Range<usize>,
    key: Key,
    width: usize,
    use_tabs: bool,
    smart_lists: bool,
) -> Option<Edit> {
    let width = width.clamp(2, 8);
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    if matches!(key, Key::Indent | Key::Outdent) {
        return indentation::lines(
            text,
            selection,
            width,
            use_tabs,
            matches!(key, Key::Outdent),
        );
    }
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let end = text[selection.start..]
        .find('\n')
        .map_or(text.len(), |i| selection.start + i);
    let line = text[start..end].trim_end_matches('\r');
    let p = if matches!(key, Key::Enter) {
        continuation_prefix(text, start, line)
    } else {
        prefix(line)
    }?;
    // Ordinary Backspace within list text must not parse the document.
    match key {
        Key::Backspace if !selection.is_empty() || selection.start != start + p.end => return None,
        Key::Enter | Key::SoftEnter
            if selection.start < start + p.end || selection.end > start + line.len() =>
        {
            return None;
        }
        _ => {}
    }
    let literals = literal_ranges(text);
    let literal_at = |offset| {
        literals
            .iter()
            .any(|r| r.start <= offset && offset <= r.end)
    };
    if literal_at(start + p.end) {
        return None;
    }
    let after = |range: Range<usize>, replacement: String| {
        let cursor = range.start + replacement.len();
        Edit {
            range,
            replacement,
            selection: cursor..cursor,
        }
    };
    match key {
        Key::SoftEnter => {
            if !selection.is_empty() {
                return None;
            }
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let marker = &line[p.quote.len() + p.indent.len()..p.end];
            let spaces: usize = marker
                .chars()
                .map(|ch| if ch == '\t' { 4 } else { 1 })
                .sum();
            Some(after(
                selection,
                format!("{newline}{}{}{}", p.quote, p.indent, " ".repeat(spaces)),
            ))
        }
        Key::Enter => {
            if selection.is_empty() && line[p.end..].trim().is_empty() {
                let replacement = if !p.indent.is_empty() && !p.marker.is_empty() {
                    format!(
                        "{}{}{}{}{}",
                        p.quote,
                        outdent(p.indent, width),
                        p.marker,
                        p.gap,
                        if p.task { "[ ] " } else { "" }
                    )
                } else if !p.marker.is_empty() {
                    p.quote.to_string()
                } else {
                    // Leave one quote level at a time.
                    p.quote[..p.quote.rfind('>')?].to_string()
                };
                let edit = after(start..start + line.len(), replacement);
                return Some(
                    if smart_lists && p.indent.is_empty() && p.marker.ends_with(['.', ')']) {
                        renumber::removed_item(text, edit)
                    } else {
                        edit
                    },
                );
            }
            let marker = if p.marker.ends_with(['.', ')']) {
                let n = p.marker[..p.marker.len() - 1].parse::<u32>().ok()?;
                if n == 999_999_999 {
                    return None;
                }
                format!("{}{}", n + 1, &p.marker[p.marker.len() - 1..])
            } else {
                p.marker.to_string()
            };
            let newline = if text[start..end].ends_with('\r')
                || (end == text.len() && text[..start].contains("\r\n"))
            {
                "\r\n"
            } else {
                "\n"
            };
            let tail = prefix(&line[selection.end - start..]).filter(|p| !p.marker.is_empty());
            let edit = if let Some(tail) = tail {
                after(
                    selection.start..selection.end + tail.quote.len() + tail.indent.len(),
                    format!("{newline}{}{}", p.quote, p.indent),
                )
            } else {
                after(
                    selection,
                    format!(
                        "{newline}{}{}{marker}{}{}",
                        p.quote,
                        p.indent,
                        p.gap,
                        if p.task { "[ ] " } else { "" }
                    ),
                )
            };
            Some(if smart_lists && p.marker.ends_with(['.', ')']) {
                renumber::following_items(text, edit)
            } else {
                edit
            })
        }
        Key::Backspace => {
            let replacement = if !p.indent.is_empty() {
                format!(
                    "{}{}{}",
                    p.quote,
                    outdent(p.indent, width),
                    &line[p.quote.len() + p.indent.len()..p.end]
                )
            } else if !p.marker.is_empty() {
                p.quote.to_string()
            } else {
                p.quote[..p.quote.rfind('>')?].to_string()
            };
            let edit = after(start..start + p.end, replacement);
            Some(
                if smart_lists && p.indent.is_empty() && p.marker.ends_with(['.', ')']) {
                    renumber::removed_item(text, edit)
                } else {
                    edit
                },
            )
        }
        Key::Indent | Key::Outdent => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enter_recovers_list_markers_from_continuations_and_reuses_existing_markers() {
        assert_eq!(
            press("- first\n  continuation|", Key::Enter),
            "- first\n  continuation\n- |"
        );
        assert_eq!(
            press("1. first\n   more\n   text|\n2. next", Key::Enter),
            "1. first\n   more\n   text\n2. |\n3. next"
        );
        assert_eq!(
            press("> - [x] first\n>       text|", Key::Enter),
            "> - [x] first\n>       text\n> - [ ] |"
        );
        assert_eq!(press("- first|  - next", Key::Enter), "- first\n|- next");
        assert_eq!(
            press("> - first|> - next", Key::Enter),
            "> - first\n> |- next"
        );
        assert_eq!(
            press("- first\n\n  text|", Key::Enter),
            "- first\n\n  text|"
        );
        assert_eq!(
            press("```\n- first\n  text|\n```", Key::Enter),
            "```\n- first\n  text|\n```"
        );
    }
    #[test]
    fn indentation_handles_paragraphs_quotes_mixed_whitespace_and_selection_edges() {
        assert_eq!(press("中|文", Key::Indent), "    中|文");
        assert_eq!(press("> 中|文", Key::Indent), ">     中|文");
        assert_eq!(press("  > 中|文", Key::Outdent), "> 中|文");
        assert_eq!(press(" \t  中|文", Key::Outdent), "  中|文");
        assert_eq!(press("text  |here", Key::Outdent), "text  |here");
        assert_eq!(
            press("- parent|\n    - child", Key::Indent),
            "    - parent|\n    - child"
        );
        let text = "> - parent\r\n>   continuation\r\nnext";
        let end = text.find("next").unwrap();
        let edit = edit_with_indent(text, 0..end, Key::Indent, 2, false).unwrap();
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert_eq!(result, ">   - parent\r\n>     continuation\r\nnext");
        assert_eq!(edit.selection.end, result.find("next").unwrap());
        assert_eq!(press("text\n|", Key::Indent), "text\n    |");
    }
    #[test]
    fn removing_ordered_markers_closes_number_gaps() {
        assert_eq!(press("1. a\n2. |\n3. b", Key::Enter), "1. a\n|\n2. b");
        assert_eq!(press("7. |\n8. b\n9. c", Key::Enter), "|\n7. b\n8. c");
        assert_eq!(
            press("8. a\n9. |text\n10. next\n11. last", Key::Backspace),
            "8. a\n|text\n9. next\n10. last"
        );
        assert_eq!(
            press("> 1) a\r\n> 2) |\r\n> 3) b", Key::Enter),
            "> 1) a\r\n> |\r\n> 2) b"
        );
        let source = "1. a\n2. b\n3. c";
        let edit = edit_with_options(source, 8..8, Key::Backspace, 4, false, false).unwrap();
        let mut actual = source.to_string();
        actual.replace_range(edit.range, &edit.replacement);
        assert_eq!(actual, "1. a\nb\n3. c");
    }
    #[test]
    fn ordered_enter_renumbers_following_siblings_without_touching_other_blocks() {
        assert_eq!(
            press("1. 中|文\n2. two\n3. three", Key::Enter),
            "1. 中\n2. |文\n3. two\n4. three"
        );
        assert_eq!(
            press("9) a|\n10) b\n11) c", Key::Enter),
            "9) a\n10) |\n11) b\n12) c"
        );
        assert_eq!(
            press(
                "1. a|\n   1. nested\n   2. keep\n2. b\n\nparagraph\n\n1. separate",
                Key::Enter
            ),
            "1. a\n2. |\n   1. nested\n   2. keep\n3. b\n\nparagraph\n\n1. separate"
        );
        assert_eq!(
            press("> 1. a|\r\n> 2. b", Key::Enter),
            "> 1. a\r\n> 2. |\r\n> 3. b"
        );
        assert_eq!(
            press("1. parent\n   1. a|\n   2. b\n2. end", Key::Enter),
            "1. parent\n   1. a\n   2. |\n   3. b\n2. end"
        );
        assert_eq!(
            press("1. a|\n2. b\n\n```\n3. literal\n```", Key::Enter),
            "1. a\n2. |\n3. b\n\n```\n3. literal\n```"
        );
    }
    #[test]
    fn soft_enter_keeps_list_content_indentation_without_a_new_marker() {
        for (source, expected) in [
            ("- 中文", "- 中文\n  "),
            ("> 12. [x] text", "> 12. [x] text\n>         "),
            ("> quote", "> quote\n> "),
            ("a\r\n- text", "a\r\n- text\r\n  "),
        ] {
            let edit = edit(source, source.len()..source.len(), Key::SoftEnter).unwrap();
            let mut actual = source.to_string();
            actual.replace_range(edit.range, &edit.replacement);
            assert_eq!(actual, expected);
            assert_eq!(edit.selection, expected.len()..expected.len());
        }
    }
    #[test]
    fn configured_indentation_preserves_unicode_quotes_and_line_endings() {
        let apply = |text: &str, selection, key, width, tabs| {
            let edit = edit_with_indent(text, selection, key, width, tabs)
                .unwrap_or_else(|| panic!("no edit for {text:?}"));
            let mut result = text.to_string();
            result.replace_range(edit.range, &edit.replacement);
            assert!(result.is_char_boundary(edit.selection.start));
            assert!(result.is_char_boundary(edit.selection.end));
            (result, edit.selection)
        };
        let text = "> - 中文😀\r\n> - second";
        let (indented, selected) = apply(text, 0..text.len(), Key::Indent, 2, false);
        assert_eq!(indented, ">   - 中文😀\r\n>   - second");
        assert_eq!(apply(&indented, selected, Key::Outdent, 2, false).0, text);
        let text = "- parent\n- 中文\n- second";
        let (indented, selected) = apply(text, 9..text.len(), Key::Indent, 8, true);
        assert_eq!(indented, "- parent\n\t- 中文\n\t- second");
        assert_eq!(apply(&indented, selected, Key::Outdent, 8, true).0, text);
        assert_eq!(apply("  - ", 4..4, Key::Enter, 2, false).0, "- ");
        assert_eq!(
            apply("- parent\n      - text", 17..17, Key::Backspace, 6, false).0,
            "- parent\n- text"
        );
    }
    #[test]
    fn footnote_insertion_preserves_selection_text_and_uses_next_definition_id() {
        let text = "中文😀\n\n[^02]: existing\n\n`[^99]: code`\n\n```\n[^100]: code\n```\n";
        let edit = insert_footnote(text, 3..10).unwrap();
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.replacement);
        assert_eq!(result, format!("中[^3]{}\n[^3]: \n", &text[3..]));
        assert_eq!(edit.selection, 7..7);
        assert!(insert_footnote(text, 1..3).is_none());
        assert!(insert_footnote(text, 0..text.len() + 1).is_none());
    }
    #[test]
    fn footnote_insertion_handles_empty_crlf_and_large_ids() {
        assert_eq!(
            insert_footnote("", 0..0).unwrap().replacement,
            "[^1]\n\n[^1]: \n"
        );
        let text = "中文\r\n\r\n[^99999999999999999999999]: a\r\n\r\n";
        let edit = insert_footnote(text, 0..0).unwrap();
        assert_eq!(
            edit.replacement,
            format!("[^100000000000000000000000]{text}[^100000000000000000000000]: \r\n")
        );
        let edit = insert_footnote("a\n\n", 3..3).unwrap();
        assert_eq!(edit.replacement, "[^1]\n\n[^1]: \n");
        let first = insert_footnote("", 0..0).unwrap();
        let second = insert_footnote(&first.replacement, first.selection).unwrap();
        assert!(second.replacement.starts_with("[^2]"));
    }
    #[test]
    fn link_insertion_positions_the_caret_like_the_reference() {
        let text = "中文😀";
        let edit = insert_link(text, 0..text.len(), false).unwrap();
        assert_eq!(edit.replacement, "[中文😀]()");
        assert_eq!(edit.selection, 13..13);
        let edit = insert_link("", 0..0, false).unwrap();
        assert_eq!(edit.replacement, "[]()");
        assert_eq!(edit.selection, 1..1);
        let edit = insert_link(text, 0..text.len(), true).unwrap();
        assert_eq!(edit.replacement, "[[中文😀]]");
        assert_eq!(edit.selection, 12..12);
    }
    #[test]
    fn block_commands_preserve_crlf_selection_and_fenced_content() {
        let apply = |text: &str, selection: Range<usize>, kind: BlockFormat| {
            let edit = block_format(text, selection, kind).unwrap();
            let mut out = text.to_string();
            out.replace_range(edit.range, &edit.replacement);
            (out, edit.selection)
        };
        let text = "中文\r\nemoji😀\r\nnext";
        let end = text.find("next").unwrap();
        let (out, selected) = apply(text, 0..end, BlockFormat::Numbered);
        assert_eq!(out, "1. 中文\r\n2. emoji😀\r\nnext");
        assert_eq!(&out[selected], "中文\r\n2. emoji😀\r\n");
        let (out, _) = apply("- a\n- b", 0..7, BlockFormat::Bullet);
        assert_eq!(out, "a\nb");
        let (out, _) = apply("## title", 3..8, BlockFormat::Heading(1));
        assert_eq!(out, "# title");
        let (out, _) = apply("# title", 2..7, BlockFormat::Heading(0));
        assert_eq!(out, "title");
        assert_eq!(apply("# title", 2..7, BlockFormat::Bullet).0, "- title");
        assert_eq!(apply("> a\n> b", 0..7, BlockFormat::Quote).0, "a\nb");
        assert_eq!(
            apply("a\n\nb", 0..4, BlockFormat::Numbered).0,
            "1. a\n\n2. b"
        );
        let text = "```rust\ncode\n```";
        let (out, selected) = apply(text, 0..text.len(), BlockFormat::Code);
        assert_eq!(&out[selected], text);
        assert!(out.starts_with("````\n"));
        assert!(out.ends_with("\n````"));
        assert_eq!(
            apply("", 0..0, BlockFormat::Code),
            ("```\n\n```".into(), 4..4)
        );
        let (out, selection) = apply("a\r\nb", 0..4, BlockFormat::Callout);
        assert_eq!(out, "> [!note]\r\n> a\r\n> b");
        assert_eq!(&out[selection], "a\r\n> b");
    }
    #[test]
    fn inline_toggles_respect_nested_styles_and_backtick_delimiters() {
        let apply = |text: &str, range: Range<usize>, marker: &str| {
            let edit = inline_format(text, range, marker, marker).unwrap();
            let mut text = text.to_string();
            text.replace_range(edit.range, &edit.replacement);
            (text, edit.selection)
        };
        assert_eq!(apply("**中文**", 2..8, "**"), ("中文".into(), 0..6));
        assert_eq!(apply("**text**", 2..6, "*"), ("***text***".into(), 3..7));
        assert_eq!(apply("***text***", 3..7, "*"), ("**text**".into(), 2..6));
        let (text, selected) = apply("a`b", 0..3, "`");
        assert_eq!(text, "``a`b``");
        assert_eq!(&text[selected], "a`b");
        assert_eq!(apply("==text==", 0..8, "==").0, "text");
        assert_eq!(apply("~~text~~", 2..6, "~~").0, "text");
        assert_eq!(apply("****", 2..2, "**"), (String::new(), 0..0));
    }
    fn press(source: &str, key: Key) -> String {
        let cursor = source.find('|').unwrap();
        let mut text = source.replacen('|', "", 1);
        if let Some(e) = edit(&text, cursor..cursor, key) {
            text.replace_range(e.range, &e.replacement);
            text.insert(e.selection.end, '|');
        } else {
            text.insert(cursor, '|');
        }
        text
    }
    #[test]
    fn continues_lists_quotes_tasks_and_crlf() {
        assert_eq!(press("- 中文|尾部", Key::Enter), "- 中文\n- |尾部");
        assert_eq!(press("9) 第九项|", Key::Enter), "9) 第九项\n10) |");
        assert_eq!(
            press("> - [X] 完成|", Key::Enter),
            "> - [X] 完成\n> - [ ] |"
        );
        assert_eq!(press("> > 引用|", Key::Enter), "> > 引用\n> > |");
        assert_eq!(
            press("- 第一项\r\n- 第二项|", Key::Enter),
            "- 第一项\r\n- 第二项\r\n- |"
        );
    }
    #[test]
    fn exits_empty_items_one_level_at_a_time() {
        assert_eq!(press("- [ ] |", Key::Enter), "|");
        assert_eq!(press("- 父\n    - |", Key::Enter), "- 父\n- |");
        assert_eq!(press("> - |", Key::Enter), "> |");
        assert_eq!(press("> > |", Key::Enter), "> |");
    }
    #[test]
    fn ignores_literal_blocks_and_marker_interiors() {
        for text in [
            "```md\n- literal|\n```",
            "---\n- yaml|\n---",
            "***|",
            "- - | -",
            "    - code|",
            "1|. text",
        ] {
            assert_eq!(press(text, Key::Enter), text);
        }
    }
    #[test]
    fn prefix_backspace_and_indent_preserve_unicode() {
        assert_eq!(press("- |中文👩‍💻", Key::Backspace), "|中文👩‍💻");
        assert_eq!(press("- 父\n    - |子", Key::Backspace), "- 父\n- |子");
        assert_eq!(press("> - 中|文", Key::Indent), ">     - 中|文");
        assert_eq!(press("- 父\n    - 中|文", Key::Outdent), "- 父\n- 中|文");
        let text = "- 中文\n- emoji 👩‍💻\n正文";
        let end = text.find("正文").unwrap();
        let e = edit(text, 0..end, Key::Indent).unwrap();
        let mut result = text.to_string();
        result.replace_range(e.range, &e.replacement);
        assert_eq!(result, "    - 中文\n    - emoji 👩‍💻\n正文");
        assert_eq!(&result[e.selection], "- 中文\n    - emoji 👩‍💻\n");
    }
}
