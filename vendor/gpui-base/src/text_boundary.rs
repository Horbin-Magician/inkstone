use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterKind {
    Word,
    Whitespace,
    Newline,
    Other,
}

// Character grouping follows the original Zed line-wrapper policy:
// https://github.com/zed-industries/zed/blob/main/crates/gpui/src/text_system/line_wrapper.rs
impl From<char> for CharacterKind {
    fn from(character: char) -> Self {
        if character == '_'
            || character.is_ascii_alphanumeric()
            || matches!(character, '\u{00C0}'..='\u{024F}' | '\u{0400}'..='\u{04FF}' | '\u{1E00}'..='\u{1EFF}' | '\u{0300}'..='\u{036F}')
        {
            Self::Word
        } else if matches!(character, '\n' | '\r') {
            Self::Newline
        } else if character.is_whitespace() {
            Self::Whitespace
        } else {
            Self::Other
        }
    }
}

pub(crate) fn word_range_from_chars(
    offset: usize,
    character: char,
    previous: impl Iterator<Item = char>,
    following: impl Iterator<Item = char>,
) -> Range<usize> {
    let kind = CharacterKind::from(character);
    let connects = |next| {
        matches!(
            (kind, CharacterKind::from(next)),
            (CharacterKind::Word, CharacterKind::Word)
                | (CharacterKind::Whitespace, CharacterKind::Whitespace)
        )
    };
    let start = previous
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset, |offset, character| offset - character.len_utf8());
    let end = following
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset + character.len_utf8(), |offset, character| {
            offset + character.len_utf8()
        });
    start..end
}

pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = clip_offset_left(text, offset);
    let character = text[offset..].chars().next()?;
    let end = offset + character.len_utf8();
    Some(word_range_from_chars(
        offset,
        character,
        text[..offset].chars().rev(),
        text[end..].chars(),
    ))
}

pub(crate) fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clip_offset_left(text, offset);
    let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |newline| offset + newline);
    start..end
}

fn clip_offset_left(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_type_from_char() {
        assert_eq!(CharacterKind::from('a'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('Z'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('0'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('_'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('.'), CharacterKind::Other);
        assert_eq!(CharacterKind::from(','), CharacterKind::Other);
        assert_eq!(CharacterKind::from(';'), CharacterKind::Other);
        assert_eq!(CharacterKind::from('!'), CharacterKind::Other);
        assert_eq!(CharacterKind::from('?'), CharacterKind::Other);
        assert_eq!(CharacterKind::from('['), CharacterKind::Other);
        assert_eq!(CharacterKind::from('{'), CharacterKind::Other);
        assert_eq!(CharacterKind::from(' '), CharacterKind::Whitespace);
        assert_eq!(CharacterKind::from('\t'), CharacterKind::Whitespace);
        assert_eq!(CharacterKind::from('\u{00A0}'), CharacterKind::Whitespace);
        assert_eq!(CharacterKind::from('\n'), CharacterKind::Newline);
        assert_eq!(CharacterKind::from('\r'), CharacterKind::Newline);
        assert_eq!(CharacterKind::from('汉'), CharacterKind::Other);
        assert_eq!(CharacterKind::from('é'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('ä'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('ö'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('ü'), CharacterKind::Word);
        assert_eq!(CharacterKind::from('д'), CharacterKind::Word);
    }

    #[test]
    fn word_range_handles_utf8_and_end_offsets() {
        assert_eq!(word_range_at("", 0), None);
        assert_eq!(word_range_at("a中é", 2), Some(1..4));
        assert_eq!(word_range_at("a中é", 5), Some(4..6));
        assert_eq!(word_range_at("a中é", 6), None);
        assert_eq!(word_range_at("a中é", usize::MAX), None);
        let long_word = "é".repeat(300);
        assert_eq!(word_range_at(&long_word, 300), Some(44..558));
    }
}
