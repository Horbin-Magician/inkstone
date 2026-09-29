use std::ops::Range;

/// Move selected blocks of logical lines, retaining newline styles by position.
pub fn move_lines(
    text: &str,
    selections: &[Range<usize>],
    down: bool,
) -> Option<(String, Vec<Range<usize>>)> {
    if selections.is_empty()
        || selections.iter().any(|r| {
            r.start > r.end || !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end)
        })
    {
        return None;
    }
    let mut lines = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let (body, separator) = if let Some(body) = raw.strip_suffix("\r\n") {
            (body, "\r\n")
        } else if let Some(body) = raw.strip_suffix('\n') {
            (body, "\n")
        } else {
            (raw, "")
        };
        lines.push((body, separator, offset));
        offset += raw.len();
    }
    if text.is_empty() || text.ends_with('\n') {
        lines.push(("", "", offset));
    }
    let row_at = |offset| {
        lines
            .partition_point(|line| line.2 <= offset)
            .saturating_sub(1)
    };
    let spans: Vec<_> = selections
        .iter()
        .map(|range| {
            let first = row_at(range.start);
            let mut last = row_at(range.end);
            let through_newline = !range.is_empty() && last > first && range.end == lines[last].2;
            if through_newline {
                last -= 1;
            }
            (first, last, through_newline)
        })
        .collect();
    let mut groups: Vec<_> = spans
        .iter()
        .map(|&(first, last, _)| first..last + 1)
        .collect();
    groups.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for group in groups {
        if let Some(last) = merged.last_mut().filter(|last| group.start <= last.end) {
            last.end = last.end.max(group.end);
        } else {
            merged.push(group);
        }
    }
    let mut order: Vec<_> = (0..lines.len()).collect();
    for group in merged {
        if down && group.end < lines.len() {
            order[group.start..=group.end].rotate_right(1);
        } else if !down && group.start > 0 {
            order[group.start - 1..group.end].rotate_left(1);
        }
    }
    if order
        .iter()
        .enumerate()
        .all(|(index, &original)| index == original)
    {
        return None;
    }
    let mut result = String::with_capacity(text.len());
    let mut positions = vec![0; lines.len()];
    let mut rows = vec![0; lines.len()];
    for (row, &original) in order.iter().enumerate() {
        positions[original] = result.len();
        rows[original] = row;
        result.push_str(lines[original].0);
        result.push_str(lines[row].1);
    }
    let mapped = selections
        .iter()
        .zip(spans)
        .map(|(range, (first, last, through_newline))| {
            let start = positions[first] + range.start - lines[first].2;
            let end = positions[last]
                + if through_newline {
                    lines[last].0.len() + lines[rows[last]].1.len()
                } else {
                    range.end - lines[last].2
                };
            start.min(result.len())..end.min(result.len())
        })
        .collect();
    Some((result, mapped))
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // Fixtures contain selection ranges, not integer sequences.
mod tests {
    use super::*;
    #[test]
    fn moves_unicode_blocks_and_excludes_the_next_line_boundary() {
        assert_eq!(
            move_lines("甲\r\n乙\r\n丙", &[0..5], true).unwrap(),
            ("乙\r\n甲\r\n丙".into(), vec![5..10])
        );
        assert_eq!(
            move_lines("甲\n乙\n丙", &[0..8], true).unwrap(),
            ("丙\n甲\n乙".into(), vec![4..11])
        );
        assert!(move_lines("甲\n乙", &[0..0], false).is_none());
        assert!(move_lines("甲\n乙", &[4..4], true).is_none());
    }
    #[test]
    fn moves_disjoint_groups_once_and_maps_identical_lines() {
        assert_eq!(
            move_lines("a\nb\nc\nd\ne", &[2..2, 6..6], false).unwrap(),
            ("b\na\nd\nc\ne".into(), vec![0..0, 4..4])
        );
        assert_eq!(
            move_lines("a\na", &[0..0], true).unwrap(),
            ("a\na".into(), vec![2..2])
        );
        assert_eq!(
            move_lines("a\nb\n", &[2..2], true).unwrap(),
            ("a\n\nb".into(), vec![3..3])
        );
    }
}
