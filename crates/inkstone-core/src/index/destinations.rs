//! Locate the raw destination in an already validated Markdown link or definition.
use std::ops::Range;

pub(super) fn source_range(raw: &str, definition: bool) -> Option<Range<usize>> {
    let bytes = raw.as_bytes();
    let mut cursor = usize::from(bytes.first() == Some(&b'!'));
    if definition {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
    }
    if bytes.get(cursor) != Some(&b'[') {
        return None;
    }
    cursor += 1;
    let mut brackets = 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'`' if !definition => {
                let count = bytes[cursor..].iter().take_while(|b| **b == b'`').count();
                cursor += count;
                let mut next = cursor;
                while next < bytes.len() {
                    if bytes[next] != b'`' {
                        next += 1;
                        continue;
                    }
                    let close = bytes[next..].iter().take_while(|b| **b == b'`').count();
                    next += close;
                    if close == count {
                        cursor = next;
                        break;
                    }
                }
            }
            b'[' => {
                brackets += 1;
                cursor += 1;
            }
            b']' => {
                brackets -= 1;
                cursor += 1;
                if brackets == 0 {
                    break;
                }
            }
            _ => cursor += 1,
        }
    }
    let delimiter = if definition { b':' } else { b'(' };
    if brackets != 0 || bytes.get(cursor) != Some(&delimiter) {
        return None;
    }
    cursor += 1;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let angle = bytes.get(cursor) == Some(&b'<');
    cursor += usize::from(angle);
    let start = cursor;
    let mut parentheses = 0;
    while let Some(&byte) = bytes.get(cursor) {
        if byte == b'\\' && bytes.get(cursor + 1).is_some_and(u8::is_ascii_punctuation) {
            cursor += 2;
            continue;
        }
        if angle {
            if byte == b'>' {
                return Some(start..cursor);
            }
        } else {
            if byte.is_ascii_whitespace() || (byte == b')' && parentheses == 0) {
                break;
            }
            if byte == b'(' {
                parentheses += 1;
            }
            if byte == b')' {
                parentheses -= 1;
            }
        }
        cursor += 1;
    }
    (!angle && cursor > start).then_some(start..cursor)
}
