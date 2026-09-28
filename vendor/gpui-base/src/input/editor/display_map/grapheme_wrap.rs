use unicode_segmentation::UnicodeSegmentation;

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
