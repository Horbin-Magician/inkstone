# Inkstone patches to markdown 1.0.0

The crate is the crates.io 1.0.0 package. Its MIT license stays in `license`.

`src/construct/attention.rs` uses `src/util/cjk.rs` when deciding whether `*`, `_`, and `~` sequences can open or close. CommonMark treats CJK punctuation as ordinary punctuation, so a closing `**` after `，` or `。` is not right-flanking when the next character is a letter. Chinese text has no space there, and `**叶变换对，**这种` stayed literal. A CJK character on either side of the delimiter now counts as flanking context. Sequences that contain no CJK character keep the previous CommonMark result, including `**hello,**world` and underscore runs inside a word.
