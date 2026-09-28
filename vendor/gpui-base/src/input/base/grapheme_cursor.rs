//! Inkstone patch: cursor endpoints snap to extended graphemes without flattening
//! the rope in the normal path. Exact source and IME conversions stay unchanged.
use ropey::Rope;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

fn is_boundary(text: &Rope, offset: usize) -> bool {
    if offset == 0 || offset == text.len() {
        return true;
    }
    let (chunk, start) = text.chunk(offset);
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    loop {
        match cursor.is_boundary(chunk, start) {
            Ok(boundary) => return boundary,
            Err(GraphemeIncomplete::PreContext(end)) if end > 0 => {
                let (context, base) = text.chunk(end - 1);
                cursor.provide_context(&context[..end - base], base);
            }
            // Defensive fallback for a future Rope/GraphemeCursor API change.
            Err(_) => {
                return text
                    .to_string()
                    .grapheme_indices(true)
                    .any(|(i, _)| i == offset);
            }
        }
    }
}

pub(super) fn snap_grapheme(text: &Rope, offset: usize, forward: bool) -> usize {
    let mut offset = offset.min(text.len());
    let mut scalar = text.byte_to_char_idx(offset);
    while !is_boundary(text, offset) {
        scalar = if forward {
            scalar + 1
        } else {
            scalar.saturating_sub(1)
        };
        offset = text.char_to_byte_idx(scalar);
    }
    offset
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_snap_matches_full_unicode_segmentation_across_rope_chunks() {
        for source in [
            "中👩‍💻e\u{301}\r\n🇨🇳".repeat(300),
            "🇨".repeat(1100),
            format!("e{}Z", "\u{301}".repeat(3000)),
        ] {
            let rope = Rope::from(source.as_str());
            assert!(rope.chunks().count() > 1);
            let boundaries: Vec<_> = source
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(source.len()))
                .collect();
            for offset in source
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(source.len()))
            {
                let before = boundaries.partition_point(|i| *i <= offset) - 1;
                let after = boundaries.partition_point(|i| *i < offset);
                assert_eq!(
                    snap_grapheme(&rope, offset, false),
                    boundaries[before],
                    "left at {offset}"
                );
                assert_eq!(
                    snap_grapheme(&rope, offset, true),
                    boundaries[after],
                    "right at {offset}"
                );
            }
        }
    }
}
