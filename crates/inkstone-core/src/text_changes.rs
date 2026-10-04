//! Bounded text differences for mapping selections in another view of a document.
use similar::{ChangeTag, TextDiff};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

fn collect<'a>(diff: &TextDiff<'a, 'a, 'a, str>, start: usize) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    let mut pending: Option<(Range<usize>, String)> = None;
    let mut offset = start;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                if let Some(edit) = pending.take() {
                    edits.push(edit);
                }
                offset += change.value().len();
            }
            ChangeTag::Delete => {
                let edit = pending.get_or_insert_with(|| (offset..offset, String::new()));
                offset += change.value().len();
                edit.0.end = offset;
            }
            ChangeTag::Insert => {
                pending
                    .get_or_insert_with(|| (offset..offset, String::new()))
                    .1
                    .push_str(change.value());
            }
        }
    }
    if let Some(edit) = pending {
        edits.push(edit);
    }
    edits
}

pub fn diff(before: &str, after: &str) -> Vec<(Range<usize>, String)> {
    if before == after {
        return Vec::new();
    }
    let start = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = before[start..]
        .chars()
        .rev()
        .zip(after[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let old = &before[start..before.len() - suffix];
    let new = &after[start..after.len() - suffix];
    let deadline = Instant::now() + Duration::from_millis(20);
    let line_diff = TextDiff::configure()
        .deadline(deadline)
        .diff_lines(old, new);
    let mut result = Vec::new();
    for (range, replacement) in collect(&line_diff, start) {
        if range.len() + replacement.len() <= 65536 && Instant::now() < deadline {
            let chars = TextDiff::configure()
                .deadline(deadline)
                .diff_chars(&before[range.clone()], replacement.as_str());
            result.extend(collect(&chars, range.start));
        } else {
            result.push((range, replacement));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_edits_preserve_unicode_and_unchanged_text() {
        assert_eq!(
            diff("aX\nbX", "a\nb"),
            vec![(1..2, String::new()), (4..5, String::new())]
        );
        for (before, after) in [
            ("中文😀 hello", "中文😀 world"),
            ("a😀b", "a👩‍💻b"),
            ("a\r\nb", "aX\r\nbX"),
            ("", "中文"),
            ("a", ""),
        ] {
            let mut result = before.to_string();
            for (range, replacement) in diff(before, after).into_iter().rev() {
                result.replace_range(range, &replacement);
            }
            assert_eq!(result, after);
        }
    }
}
