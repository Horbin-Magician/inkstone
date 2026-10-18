use std::ops::Range;

/// Inline text keeps selecting the final word when hit testing reaches the end.
pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = if offset >= text.len() {
        text.char_indices().next_back()?.0
    } else {
        offset
    };
    crate::text_boundary::word_range_at(text, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_range_preserves_end_selection_and_utf8_clipping() {
        assert_eq!(word_range_at("", usize::MAX), None);
        assert_eq!(word_range_at("a中é", 2), Some(1..4));
        assert_eq!(word_range_at("a中é", 5), Some(4..6));
        assert_eq!(word_range_at("a中é", 6), Some(4..6));
        assert_eq!(word_range_at("a中é", usize::MAX), Some(4..6));
    }

    #[test]
    fn test_word_range_at() {
        let text =
            "test text\nabcde 中文🎉 test\nhello[()]\ntest_connector ____\nRope\nrök\ngrande île";
        let tests = [
            (0, Some("test")),
            (4, Some(" ")),
            (10, Some("abcde")),
            (15, Some(" ")),
            (16, Some("中")),
            (19, Some("文")),
            (22, Some("🎉")),
            (27, Some("test")),
            (37, Some("[")),
            (38, Some("(")),
            (39, Some(")")),
            (40, Some("]")),
            (42, Some("test_connector")),
            (56, Some(" ")),
            (57, Some("____")),
            (62, Some("Rope")),
            (67, Some("rök")),
            (79, Some("île")),
        ];

        for (offset, expected) in tests {
            let actual = word_range_at(text, offset).map(|range| text[range].to_string());
            assert_eq!(actual.as_deref(), expected, "offset {offset}");
        }
    }
}
