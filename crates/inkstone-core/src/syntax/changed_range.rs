//! Locate an edit once, comparing unchanged bytes in chunks before restoring
//! Unicode scalar boundaries for Markdown's source coordinates.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChangedRange {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl ChangedRange {
    pub fn between(before: &str, after: &str) -> Self {
        const CHUNK: usize = 64;
        let a = before.as_bytes();
        let b = after.as_bytes();
        let equal = a
            .chunks_exact(CHUNK)
            .zip(b.chunks_exact(CHUNK))
            .take_while(|(a, b)| a == b)
            .count()
            * CHUNK;
        let mut start = equal
            + a[equal..]
                .iter()
                .zip(&b[equal..])
                .take_while(|(a, b)| a == b)
                .count();
        // Different scalars may share leading UTF-8 bytes (e.g. two emoji).
        while !before.is_char_boundary(start) {
            start -= 1;
        }
        let a = &a[start..];
        let b = &b[start..];
        let equal = a
            .rchunks_exact(CHUNK)
            .zip(b.rchunks_exact(CHUNK))
            .take_while(|(a, b)| a == b)
            .count()
            * CHUNK;
        let mut suffix = equal
            + a[..a.len() - equal]
                .iter()
                .rev()
                .zip(b[..b.len() - equal].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
        // Equal trailing bytes can also start inside different scalars. Keep
        // the full changed scalar, and never overlap the unchanged prefix.
        while !before.is_char_boundary(before.len() - suffix) {
            suffix -= 1;
        }
        Self {
            start,
            old_end: before.len() - suffix,
            new_end: after.len() - suffix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_ranges_match_scalar_comparison_at_every_boundary() {
        fn scalar_range(before: &str, after: &str) -> ChangedRange {
            let start = before
                .chars()
                .zip(after.chars())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            let suffix = before[start..]
                .chars()
                .rev()
                .zip(after[start..].chars().rev())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            ChangedRange {
                start,
                old_end: before.len() - suffix,
                new_end: after.len() - suffix,
            }
        }
        for (before, after) in [("", ""), ("", "中😀"), ("中😀", ""), ("same", "same")] {
            assert_eq!(
                ChangedRange::between(before, after),
                scalar_range(before, after)
            );
        }
        // Include shared leading/trailing bytes, combining sequences, tabs,
        // CRLF, insertions, deletions, and all 64 possible chunk alignments.
        for padding in 0..64 {
            let before = format!(
                "{}{}",
                "x".repeat(padding),
                "aéê中😀😁👩‍💻 e\u{301}\r\n\t".repeat(5)
            );
            for start in before.char_indices().map(|(i, _)| i).chain([before.len()]) {
                for end in [
                    start,
                    before[start..]
                        .chars()
                        .next()
                        .map_or(start, |c| start + c.len_utf8()),
                    before.len(),
                ] {
                    for replacement in ["", "a", "ê", "😁", "中\r\ne\u{301}", "**加粗**"] {
                        let mut after = before.clone();
                        after.replace_range(start..end, replacement);
                        let range = ChangedRange::between(&before, &after);
                        assert_eq!(range, scalar_range(&before, &after));
                        let mut restored = before.clone();
                        restored.replace_range(
                            range.start..range.old_end,
                            &after[range.start..range.new_end],
                        );
                        assert_eq!(restored, after);
                    }
                }
            }
        }
    }
}
