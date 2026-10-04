use super::Edit;
use std::ops::Range;

pub(super) fn lines(
    text: &str,
    selection: Range<usize>,
    width: usize,
    tabs: bool,
    outdent: bool,
) -> Option<Edit> {
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let last = if !selection.is_empty() && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i);
    let mut changes = vec![];
    let mut offset = start;
    // split includes a final empty line, which Tab must also indent.
    for row in text[start..end].split('\n') {
        if let Some((range, replacement)) = line_change(row, width, tabs, outdent) {
            changes.push((offset + range.start..offset + range.end, replacement));
        }
        offset += row.len() + 1;
    }
    if changes.is_empty() {
        return None;
    }
    let map = |pos: usize| {
        let mut delta = 0isize;
        for (range, inserted) in &changes {
            if pos < range.start {
                break;
            }
            if pos <= range.end {
                return (range.start + inserted.len()).saturating_add_signed(delta);
            }
            delta += inserted.len() as isize - range.len() as isize;
        }
        pos.saturating_add_signed(delta)
    };
    let selected = map(selection.start)..map(selection.end);
    let mut replacement = text[start..end].to_string();
    for (range, inserted) in changes.into_iter().rev() {
        replacement.replace_range(range.start - start..range.end - start, &inserted);
    }
    Some(Edit {
        range: start..end,
        replacement,
        selection: selected,
    })
}

pub(super) fn line_change(
    row: &str,
    width: usize,
    tabs: bool,
    outdent: bool,
) -> Option<(Range<usize>, String)> {
    let prefix_end = row
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | '>'))
        .map_or(row.trim_end_matches('\r').len(), |(i, _)| i);
    let prefix = &row[..prefix_end];
    let at = prefix.rfind('>').map_or(0, |i| {
        i + 1 + usize::from(prefix.as_bytes().get(i + 1) == Some(&b' '))
    });
    if !outdent {
        return Some((at..at, if tabs { "\t".into() } else { " ".repeat(width) }));
    }
    if let Some(first) = prefix.find('>').filter(|i| *i > 0) {
        return Some((0..first, String::new()));
    }
    let indent = &prefix[at..];
    let columns = indent.chars().fold(0, |n, ch| {
        if ch == '\t' {
            n + width - n % width
        } else {
            n + 1
        }
    });
    let remaining = columns.saturating_sub(width);
    let replacement = if tabs {
        format!(
            "{}{}",
            "\t".repeat(remaining / width),
            " ".repeat(remaining % width)
        )
    } else {
        " ".repeat(remaining)
    };
    (replacement != indent).then_some((at..prefix.len(), replacement))
}
