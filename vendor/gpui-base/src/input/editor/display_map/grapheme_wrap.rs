use gpui::{Boundary, Pixels, px};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Validate the fast character-width estimate against the same shaped text
/// used for painting. Contextual CJK fallback/punctuation can make a whole row
/// wider than the sum of separately shaped characters. Reflow the remaining
/// suffix after correcting a boundary, rather than dropping or clipping it.
pub(super) fn measured_wrap_boundaries(
    text: &str,
    width: Pixels,
    atomic_ranges: &[Range<usize>],
    mut estimate: impl FnMut(Range<usize>, Pixels) -> Option<Boundary>,
    mut measure: impl FnMut(Range<usize>) -> Pixels,
) -> Vec<Boundary> {
    let ends: Vec<_> = text
        .grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .filter(|end| {
            let ix = atomic_ranges.partition_point(|r| r.end <= *end);
            !atomic_ranges.get(ix).is_some_and(|r| r.start < *end)
        })
        .collect();
    // Preserve the estimator's indent policy (in particular, a row of only
    // spaces has no continuation indent). If only shaping discovers a wrap,
    // recover the leading-space indent for a nonblank line.
    let indent = estimate(0..text.len(), width).map_or_else(
        || {
            if text.bytes().any(|c| c != b' ') {
                text.bytes().take_while(|c| *c == b' ').count().min(256)
            } else {
                0
            }
        },
        |boundary| boundary.next_indent as usize,
    );
    let indent_width = measure(0..indent);
    let mut result = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let available = (width - if start == 0 { px(0.) } else { indent_width }).max(px(1.));
        let mut budget = available;
        let first = ends.partition_point(|end| *end <= start);
        let mut end = text.len();
        loop {
            let proposed = estimate(start..text.len(), budget).map_or(text.len(), |b| start + b.ix);
            let slot = ends
                .partition_point(|end| *end <= proposed)
                .saturating_sub(1)
                .max(first);
            end = end.min(ends[slot]);
            let actual = measure(start..end);
            if actual <= available || end == ends[first] {
                break;
            }
            // The estimator can return the same boundary at a smaller budget.
            // Require progress while retaining graphemes and inline objects.
            budget = (budget - (actual - available)).max(px(1.));
            let previous = ends.partition_point(|candidate| *candidate < end) - 1;
            end = ends[previous];
        }
        if end < text.len() {
            result.push(Boundary {
                ix: end,
                next_indent: indent as u32,
            });
        }
        start = end;
    }
    result
}

// Inkstone patch: the GPUI width estimator may break inside a ZWJ/combining
// sequence. Keep the editor's display rows on source grapheme boundaries.
pub(super) fn grapheme_safe_boundaries(
    text: &str,
    boundaries: Vec<gpui::Boundary>,
) -> Vec<gpui::Boundary> {
    let starts: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut previous = 0;
    boundaries
        .into_iter()
        .filter_map(|mut boundary| {
            let slot = starts
                .partition_point(|i| *i <= boundary.ix)
                .saturating_sub(1);
            boundary.ix = starts[slot];
            if boundary.ix <= previous || boundary.ix >= text.len() {
                return None;
            }
            previous = boundary.ix;
            Some(boundary)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Boundary;

    #[test]
    fn measured_wrap_reflows_contextual_cjk_widths_without_losing_text() {
        let text = "五官、脸型、身形，以及垂在自身左肩前的深色长辫和缠在辫子上的丝带。身穿连衣裙，肩带和领口有荷叶边，脚穿浅色平底鞋。";
        // Reproduce the native discrepancy: an isolated-character estimate
        // fits more characters than the same text shaped as a complete row.
        let width = px(160.);
        let boundaries = measured_wrap_boundaries(
            text,
            width,
            &[],
            |range, budget| {
                let count = (f32::from(budget) / 14.).floor() as usize;
                text[range]
                    .char_indices()
                    .nth(count.max(1))
                    .map(|(ix, _)| Boundary { ix, next_indent: 0 })
            },
            |range| px(text[range].chars().count() as f32 * 16.),
        );
        let mut start = 0;
        let mut reconstructed = String::new();
        for end in boundaries
            .iter()
            .map(|b| b.ix)
            .chain(std::iter::once(text.len()))
        {
            let row = &text[start..end];
            assert!(
                row.chars().count() <= 10,
                "shaped row exceeds its viewport: {row}"
            );
            if end < text.len() {
                assert!(
                    row.chars().count() >= 9,
                    "corrected tail must reflow into the next row"
                );
            }
            reconstructed.push_str(row);
            start = end;
        }
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn measured_wrap_preserves_oversized_graphemes_and_inline_objects() {
        let text = "👩‍💻[object]e\u{301}尾";
        let object = text.find('[').unwrap()..text.find(']').unwrap() + 1;
        let boundaries = measured_wrap_boundaries(
            text,
            px(10.),
            std::slice::from_ref(&object),
            |_, _| {
                // A deliberately bad estimate inside a multi-byte grapheme.
                Some(Boundary {
                    ix: 1,
                    next_indent: 0,
                })
            },
            |r| {
                px(if r == object {
                    40.
                } else {
                    text[r].graphemes(true).count() as f32 * 20.
                })
            },
        );
        let expected = vec!["👩‍💻".len(), object.end, text.len() - "尾".len()];
        assert_eq!(
            boundaries.iter().map(|b| b.ix).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn measured_wrap_reserves_shaped_continuation_indent() {
        let text = "    中文中文中文中文中文中文";
        let boundaries = measured_wrap_boundaries(
            text,
            px(100.),
            &[],
            |_, _| None,
            |r| {
                px(text[r]
                    .chars()
                    .map(|c| if c == ' ' { 5. } else { 20. })
                    .sum())
            },
        );
        let mut start = 0;
        for end in boundaries
            .iter()
            .map(|b| b.ix)
            .chain(std::iter::once(text.len()))
        {
            let width: f32 = text[start..end]
                .chars()
                .map(|c| if c == ' ' { 5. } else { 20. })
                .sum();
            assert!(width + if start == 0 { 0. } else { 20. } <= 100.);
            start = end;
        }
        assert!(boundaries.iter().all(|b| b.next_indent == 4));
    }

    #[test]
    fn measured_wrap_keeps_whitespace_only_rows_unindented() {
        let text = " ".repeat(20);
        let boundaries = measured_wrap_boundaries(
            &text,
            px(25.),
            &[],
            |range, _| {
                (range.len() > 5).then_some(Boundary {
                    ix: 5,
                    next_indent: 0,
                })
            },
            |range| px(range.len() as f32 * 5.),
        );
        assert_eq!(
            boundaries.iter().map(|b| b.ix).collect::<Vec<_>>(),
            vec![5, 10, 15]
        );
        assert!(boundaries.iter().all(|b| b.next_indent == 0));
    }
    #[test]
    fn inkstone_grapheme_wrap_keeps_emoji_and_combining_sequences_whole() {
        let text = "A👩‍💻e\u{301}🇨🇳Z";
        // Exercise every scalar boundary, including points inside graphemes.
        let proposals = text
            .char_indices()
            .skip(1)
            .map(|(ix, _)| Boundary { ix, next_indent: 0 })
            .collect();
        let actual: Vec<_> = grapheme_safe_boundaries(text, proposals)
            .into_iter()
            .map(|b| b.ix)
            .collect();
        let expected: Vec<_> = text
            .grapheme_indices(true)
            .skip(1)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(actual, expected);
        let mut start = 0;
        let mut reconstructed = String::new();
        for end in actual.into_iter().chain(std::iter::once(text.len())) {
            assert!(end > start);
            assert_eq!(text[start..end].graphemes(true).count(), 1);
            reconstructed.push_str(&text[start..end]);
            start = end;
        }
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn inkstone_grapheme_wider_than_row_does_not_produce_empty_rows() {
        let text = "👨‍👩‍👧‍👦";
        let proposals = text
            .char_indices()
            .map(|(ix, _)| Boundary { ix, next_indent: 2 })
            .collect();
        assert!(grapheme_safe_boundaries(text, proposals).is_empty());
    }
}
