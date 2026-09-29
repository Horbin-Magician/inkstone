//! Status-bar counts using the installed Obsidian 1.13.7 character classes.
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub characters: usize,
}

pub fn count(text: &str) -> Counts {
    static WORDS: LazyLock<regex::Regex> = LazyLock::new(|| {
        let mut classes = include_str!("word_count_ranges.txt").lines();
        let letters = classes.next().unwrap();
        let individual = classes.next().unwrap();
        regex::Regex::new(&format!(
            r"(?:[0-9]+(?:(?:,|\.)[0-9]+)*|[\-'’{letters}஀-௿가-힣ꥠ-ꥼힰ-ퟆ])+|[{individual}]"
        ))
        .unwrap()
    });
    Counts {
        words: WORDS.find_iter(text).count(),
        characters: text.encode_utf16().count(),
    }
}

pub fn document_body(source: &str) -> &str {
    let opening = if source.starts_with("---\r\n") {
        5
    } else if source.starts_with("---\n") {
        4
    } else {
        return source;
    };
    let mut offset = opening;
    for line in source[opening..].split_inclusive('\n') {
        offset += line.len();
        if matches!(line, "---" | "---\n" | "---\r\n") {
            return &source[offset..];
        }
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multilingual_numbers_and_utf16_match_reference_rules() {
        assert_eq!(
            count("你好 world123"),
            Counts {
                words: 3,
                characters: 11
            }
        );
        assert_eq!(count("don't re-enter 1,234.50").words, 3);
        assert_eq!(count("日本語 한국어").words, 4);
        assert_eq!(
            count("😀"),
            Counts {
                words: 0,
                characters: 2
            }
        );
        assert_eq!(count("foo_bar").words, 2);
    }
    #[test]
    fn full_document_excludes_only_complete_frontmatter() {
        assert_eq!(document_body("---\r\ntags: [a]\r\n---\r\n正文"), "正文");
        assert_eq!(document_body("---\ninvalid: [\n---\ntext"), "text");
        assert_eq!(document_body("---\nunfinished"), "---\nunfinished");
        assert_eq!(document_body("---\n---"), "");
        assert_eq!(document_body("---\n---\r"), "---\n---\r");
    }
}
