use super::*;
use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub page: i32,
    pub bytes: Range<usize>,
}

/// Scan one page per worker iteration so navigation and newer queries can interrupt a search.
pub(super) struct Scan {
    pub generation: u64,
    pub hits: Vec<Hit>,
    next_page: i32,
    pattern: Regex,
}

impl Scan {
    pub fn new(generation: u64, query: &str) -> Result<Self, regex::Error> {
        let pattern = RegexBuilder::new(&regex::escape(query))
            .case_insensitive(true)
            .build()?;
        Ok(Self {
            generation,
            hits: Vec::new(),
            next_page: 0,
            pattern,
        })
    }

    pub fn advance(&mut self, document: &PdfDocument<'_>) -> anyhow::Result<bool> {
        if self.next_page < document.pages().len() {
            let page = document.pages().get(self.next_page)?;
            let text = page.text()?;
            // Use the same PDFium character sequence as the visible page's text layer.
            let content: String = text
                .chars()
                .iter()
                .filter_map(|ch| ch.unicode_char())
                .collect();
            self.hits
                .extend(self.pattern.find_iter(&content).map(|m| Hit {
                    page: self.next_page,
                    bytes: m.range(),
                }));
            self.next_page += 1;
        }
        Ok(self.next_page >= document.pages().len())
    }
}
