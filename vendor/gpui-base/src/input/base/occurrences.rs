use std::{borrow::Cow, ops::Range};
use unicode_normalization::UnicodeNormalization;

struct Mapping {
    normalized: Range<usize>,
    original: Range<usize>,
}

/// Character-wise compatibility decomposition, with sparse source mappings.
pub(super) struct Occurrences<'a> {
    text: Cow<'a, str>,
    mappings: Vec<Mapping>,
}

impl<'a> Occurrences<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        if source.is_ascii() {
            return Self {
                text: Cow::Borrowed(source),
                mappings: Vec::new(),
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut mappings = Vec::new();
        for (start, ch) in source.char_indices() {
            let before = text.len();
            if ch.is_ascii() {
                text.push(ch);
            } else {
                text.extend(ch.nfkd());
            }
            let original = start..start + ch.len_utf8();
            if text[before..] != source[original.clone()] {
                mappings.push(Mapping {
                    normalized: before..text.len(),
                    original,
                });
            }
        }
        Self {
            text: Cow::Owned(text),
            mappings,
        }
    }

    fn source_offset(&self, offset: usize, end: bool) -> usize {
        let index = self
            .mappings
            .partition_point(|mapping| mapping.normalized.end <= offset);
        if let Some(mapping) = self.mappings.get(index)
            && mapping.normalized.start < offset
        {
            return if end {
                mapping.original.end
            } else {
                mapping.original.start
            };
        }
        if index == 0 {
            offset
        } else {
            let previous = &self.mappings[index - 1];
            previous.original.end + offset - previous.normalized.end
        }
    }

    pub(super) fn ranges<'b>(&'b self, query: &'b str) -> impl Iterator<Item = Range<usize>> + 'b {
        self.ranges_from(query, 0)
    }

    pub(super) fn ranges_from<'b>(
        &'b self,
        query: &'b str,
        original_start: usize,
    ) -> impl Iterator<Item = Range<usize>> + 'b {
        let index = self
            .mappings
            .partition_point(|mapping| mapping.original.end <= original_start);
        let normalized_start = if index == 0 {
            original_start
        } else {
            let previous = &self.mappings[index - 1];
            previous.normalized.end + original_start - previous.original.end
        };
        let mut previous_end = 0;
        self.text[normalized_start..]
            .match_indices(query)
            .filter_map(move |(offset, _)| {
                if query.is_empty() {
                    return None;
                }
                let start = normalized_start + offset;
                let range =
                    self.source_offset(start, false)..self.source_offset(start + query.len(), true);
                if range.start < previous_end {
                    return None;
                }
                previous_end = range.end;
                Some(range)
            })
    }
}
