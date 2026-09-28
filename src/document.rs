//! Source coordinates are UTF-8 bytes. Platform IME coordinates are UTF-16.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub fn byte_to_utf16(text: &str, byte: usize) -> Option<usize> {
    text.get(..byte).map(|s| s.encode_utf16().count())
}

pub fn utf16_to_byte(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}

pub fn previous_grapheme(text: &str, byte: usize) -> Option<usize> {
    text.get(..byte)?;
    Some(
        text.grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|i| *i < byte)
            .last()
            .unwrap_or(0),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub selection: Range<usize>,
}

#[derive(Default)]
pub struct Document {
    pub text: String,
    pub selection: Range<usize>,
    pub revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Document {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            selection: self.selection.clone(),
        }
    }
    pub fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), &'static str> {
        if self.text.get(range.clone()).is_none() {
            return Err("invalid UTF-8 range");
        }
        self.undo.push(self.snapshot());
        self.redo.clear();
        let caret = range.start + replacement.len();
        self.text.replace_range(range, replacement);
        self.selection = caret..caret;
        self.revision += 1;
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        let Some(old) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(old);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(next);
        true
    }
    fn restore(&mut self, value: Snapshot) {
        self.text = value.text;
        self.selection = value.selection;
        self.revision += 1;
    }
    pub fn accepts(&self, revision: u64) -> bool {
        self.revision == revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_roundtrip_and_surrogate_rejection() {
        let s = "中a👩‍💻e\u{301}\r\n文";
        for b in (0..=s.len()).filter(|b| s.is_char_boundary(*b)) {
            assert_eq!(utf16_to_byte(s, byte_to_utf16(s, b).unwrap()), Some(b));
        }
        assert_eq!(utf16_to_byte("😀", 1), None);
        assert_eq!(byte_to_utf16("中", 1), None);
        assert_eq!(utf16_to_byte("中", 9), None);
    }
    #[test]
    fn grapheme_movement_preserves_emoji_and_combining_marks() {
        let s = "中👩‍💻e\u{301}";
        let b = previous_grapheme(s, s.len()).unwrap();
        assert_eq!(&s[b..], "e\u{301}");
        assert_eq!(&s[previous_grapheme(s, b).unwrap()..b], "👩‍💻");
    }
    #[test]
    fn transactions_undo_redo_and_stale_result() {
        let mut d = Document::default();
        d.replace(0..0, "中文😀").unwrap();
        let first = d.snapshot();
        let rev = d.revision;
        d.replace(3..6, "English").unwrap();
        assert!(!d.accepts(rev));
        assert!(d.undo());
        assert_eq!(d.snapshot(), first);
        assert!(d.redo());
        assert_eq!(d.text, "中English😀");
        assert!(d.replace(1..2, "x").is_err());
        d.undo();
        d.replace(0..0, "新").unwrap();
        assert!(!d.redo());
    }
}
