use super::{Key, edit_in_context, literal_ranges, renumber};
use std::{collections::BTreeSet, ops::Range};

pub fn enter_at_selections(
    text: &str,
    selections: &[Range<usize>],
    width: usize,
    tabs: bool,
) -> Option<(String, Vec<Range<usize>>)> {
    if selections.iter().any(|range| !range.is_empty()) {
        return None;
    }
    let literals = literal_ranges(text);
    let mut edits = selections
        .iter()
        .enumerate()
        .map(|(index, range)| {
            edit_in_context(
                text,
                range.clone(),
                Key::Enter,
                width,
                tabs,
                false,
                Some(&literals),
            )
            .map(|edit| (index, edit))
        })
        .collect::<Option<Vec<_>>>()?;
    edits.sort_by_key(|(_, edit)| (edit.range.start, edit.range.end));
    if edits.windows(2).any(|pair| {
        pair[0].1.range.end > pair[1].1.range.start
            || pair[0].1.range.start == pair[1].1.range.start
    }) {
        return None;
    }
    let mut selected = selections.to_vec();
    let mut delta = 0isize;
    for (index, edit) in &edits {
        selected[*index] = edit.selection.start.saturating_add_signed(delta)
            ..edit.selection.end.saturating_add_signed(delta);
        delta += edit.replacement.len() as isize - edit.range.len() as isize;
    }
    let mut result = text.to_string();
    let anchors: Vec<_> = renumber::removal_anchors(text, &edits)
        .into_iter()
        .map(|(offset, number)| {
            let mut delta = 0isize;
            for (_, edit) in &edits {
                if offset < edit.range.start {
                    break;
                }
                if offset < edit.range.end {
                    return (edit.range.start.saturating_add_signed(delta), number);
                }
                delta += edit.replacement.len() as isize - edit.range.len() as isize;
            }
            (offset.saturating_add_signed(delta), number)
        })
        .collect();
    for (_, edit) in edits.into_iter().rev() {
        result.replace_range(edit.range, &edit.replacement);
    }
    let line_starts: Vec<_> = std::iter::once(0)
        .chain(result.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let mut rows: BTreeSet<_> = selected
        .iter()
        .map(|r| line_starts.partition_point(|start| *start <= r.start))
        .collect();
    let mut starts = std::collections::BTreeMap::new();
    for (offset, number) in anchors {
        let row = line_starts.partition_point(|start| *start <= offset);
        rows.insert(row);
        if let Some(number) = number {
            starts.insert(row, number);
        }
    }
    let changes = renumber::at_rows(&result, &rows, &starts);
    let map = |position: usize| {
        let mut delta = 0isize;
        for (range, replacement) in &changes {
            if position < range.start {
                break;
            }
            if position < range.end {
                return (range.start + replacement.len()).saturating_add_signed(delta);
            }
            delta += replacement.len() as isize - range.len() as isize;
        }
        position.saturating_add_signed(delta)
    };
    for range in &mut selected {
        *range = map(range.start)..map(range.end);
    }
    for (range, replacement) in changes.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    Some((result, selected))
}
