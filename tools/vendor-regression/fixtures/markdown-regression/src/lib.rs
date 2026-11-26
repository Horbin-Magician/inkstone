//! Exercises the published parser API against this repository's CJK patch.
#[cfg(test)]
mod tests {
    #[test]
    fn adjacent_cjk_punctuation_allows_emphasis() {
        for (source, expected) in [
            ("前**中文，**后", "<p>前<strong>中文，</strong>后</p>"),
            ("**「重要」**的", "<p><strong>「重要」</strong>的</p>"),
            ("*中文，*后", "<p><em>中文，</em>后</p>"),
            (
                "***中文，***后",
                "<p><em><strong>中文，</strong></em>后</p>",
            ),
            ("**hello,**这", "<p><strong>hello,</strong>这</p>"),
        ] {
            assert_eq!(markdown::to_html(source), expected, "{source}");
        }
    }
    #[test]
    fn ascii_underscore_code_and_emoji_keep_commonmark_rules() {
        for (source, expected) in [
            ("**hello,**world", "<p>**hello,**world</p>"),
            ("__中文，__后", "<p>__中文，__后</p>"),
            ("`**中文，**后`", "<p><code>**中文，**后</code></p>"),
            ("**hello,**😀", "<p><strong>hello,</strong>😀</p>"),
            ("**hello,** world", "<p><strong>hello,</strong> world</p>"),
        ] {
            assert_eq!(markdown::to_html(source), expected, "{source}");
        }
    }
    #[test]
    fn gfm_strikethrough_uses_cjk_flanking() {
        assert_eq!(
            markdown::to_html_with_options("~~删除，~~后续", &markdown::Options::gfm()).unwrap(),
            "<p><del>删除，</del>后续</p>"
        );
    }
    #[test]
    fn japanese_korean_and_fullwidth_neighbors_are_supported() {
        for neighbor in ["日", "あ", "한", "Ａ", "𠀀"] {
            let source = format!("**hello,**{neighbor}");
            assert_eq!(
                markdown::to_html(&source),
                format!("<p><strong>hello,</strong>{neighbor}</p>")
            );
        }
    }
}
