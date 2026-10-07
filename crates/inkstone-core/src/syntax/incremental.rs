//! Reparse independent top-level inline blocks and fenced-code interiors.
use super::*;
use markdown_parser::unist::Point;

impl Snapshot {
    pub fn update_block(&self, source: &str) -> Option<Self> {
        if let Some(snapshot) = self.update_plain_paragraph(source) {
            return Some(snapshot);
        }
        if !self.comments.is_empty()
            || !self.inline_footnotes.is_empty()
            || source.contains("%%")
            || source.contains("^[")
        {
            return None;
        }
        let old = self.source.as_ref();
        let start: usize = old
            .chars()
            .zip(source.chars())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum();
        let suffix: usize = old[start..]
            .chars()
            .rev()
            .zip(source[start..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum();
        let old_end = old.len() - suffix;
        let new_end = source.len() - suffix;
        let ast = self.ast.as_deref()?;
        if has_definitions(ast) {
            return None;
        }
        let Node::Root(root) = ast else {
            return None;
        };
        // Reparse the entire paragraph even when only one of its lines changed.
        // A changed paragraph must remain one complete block; its parsed line span
        // determines how later nodes move, including mixed CR/LF boundaries.
        let (index, block) = root.children.iter().enumerate().find(|(_, node)| {
            (matches!(node, Node::Paragraph(_) | Node::Heading(_))
                || fenced_interior(node, old, start, old_end))
                && node.position().is_some_and(|p| {
                    p.start.column == 1
                        && (matches!(node, Node::Paragraph(_) | Node::Code(_))
                            || p.start.line == p.end.line)
                        && p.start.offset < start
                        && old_end <= p.end.offset
                })
        })?;
        let position = block.position()?;
        let delta = new_end as isize - old_end as isize;
        let end = position.end.offset.checked_add_signed(delta)?;
        let raw = source.get(position.start.offset..end)?;
        let Node::Root(mut parsed) = parse_raw(raw)? else {
            return None;
        };
        if parsed.children.len() != 1 {
            return None;
        }
        let mut replacement = parsed.children.pop()?;
        if std::mem::discriminant(block) != std::mem::discriminant(&replacement)
            || replacement.position()?.end.offset != raw.len()
            || has_definitions(&replacement)
        {
            return None;
        }
        relocate(&mut replacement, &position.start);
        let line_delta = replacement.position()?.end.line as isize - position.end.line as isize;
        let mut ast = ast.clone();
        ast.children_mut()?.remove(index);
        shift(
            &mut ast,
            old_end,
            delta,
            position.end.line,
            line_delta,
            source,
        );
        ast.children_mut()?.insert(index, replacement);
        let source: Arc<str> = source.into();
        Some(Self {
            structural: source.clone(),
            source,
            ast: Some(Arc::new(ast)),
            comments: vec![],
            inline_footnotes: vec![],
        })
    }
}
// Only edit the body of a closed top-level fence. Both fence lines and their
// separating newlines stay intact; changed boundaries keep the full-parse path.
fn fenced_interior(node: &Node, source: &str, start: usize, end: usize) -> bool {
    if !matches!(node, Node::Code(_)) {
        return false;
    }
    let Some(p) = node.position() else {
        return false;
    };
    let raw = &source[p.start.offset..p.end.offset];
    let Some(marker @ (b'`' | b'~')) = raw.as_bytes().first().copied() else {
        return false;
    };
    let fence_len = raw.bytes().take_while(|b| *b == marker).count();
    if fence_len < 3 {
        return false;
    }
    let Some(open_end) = raw.find(['\r', '\n']) else {
        return false;
    };
    let Some(close_start) = raw.rfind(['\r', '\n']) else {
        return false;
    };
    let closing = raw[close_start + 1..].trim();
    closing.len() >= fence_len
        && closing.bytes().all(|b| b == marker)
        && p.start.offset + open_end < start
        && end <= p.start.offset + close_start
}

fn has_definitions(node: &Node) -> bool {
    matches!(node, Node::Definition(_) | Node::FootnoteDefinition(_))
        || node
            .children()
            .is_some_and(|children| children.iter().any(has_definitions))
}
fn relocate(node: &mut Node, origin: &Point) {
    if let Some(p) = node.position_mut() {
        for point in [&mut p.start, &mut p.end] {
            point.offset += origin.offset;
            point.line += origin.line - 1;
        }
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            relocate(child, origin);
        }
    }
}
fn shift(node: &mut Node, at: usize, delta: isize, line: usize, line_delta: isize, source: &str) {
    if let Some(p) = node.position_mut() {
        for point in [&mut p.start, &mut p.end] {
            let last_edited_line = point.line == line;
            if point.offset >= at {
                point.offset = point.offset.checked_add_signed(delta).unwrap();
                point.line = point.line.checked_add_signed(line_delta).unwrap();
            }
            if last_edited_line {
                let start = source[..point.offset]
                    .rfind(['\r', '\n'])
                    .map_or(0, |i| i + 1);
                point.column = source[start..point.offset].chars().fold(1, |column, c| {
                    if c == '\t' {
                        column + 4 - (column - 1) % 4
                    } else {
                        column + c.len_utf8()
                    }
                });
            }
        }
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            shift(child, at, delta, line, line_delta, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fenced_body_edits_match_full_parse_and_keep_boundaries() {
        for newline in ["\n", "\r\n", "\r"] {
            for fence in ["```", "~~~~"] {
                let before = format!(
                    "# Before{newline}{newline}{fence}rust{newline}let value = 1;{newline}中文😀{newline}{fence}{newline}{newline}## After{newline}tail"
                );
                let base = Snapshot::new(&before);
                let start = before.find("value").unwrap();
                for replacement in ["other", "中文 é 👩‍💻", "x\nnext", "", "**bold**"] {
                    let mut after = before.clone();
                    after.replace_range(start..start + 5, replacement);
                    let updated = base.update_block(&after).expect("closed fence interior");
                    let full = Snapshot::new(&after);
                    assert_eq!(updated.ast, full.ast, "{after:?}");
                    assert_eq!(
                        crate::markdown::spans_snapshot(&updated),
                        crate::markdown::spans_snapshot(&full)
                    );
                    assert_eq!(
                        crate::index::parse_snapshot(&updated).headings,
                        crate::index::parse_snapshot(&full).headings
                    );
                }
                let after = before.replacen("rust", "text", 1);
                assert!(
                    base.update_block(&after).is_none(),
                    "fence header must fall back"
                );
                let after = before.replacen("value", &format!("{newline}{fence}{newline}end"), 1);
                assert!(
                    base.update_block(&after).is_none(),
                    "new closing fence must fall back"
                );
            }
        }
    }

    #[test]
    fn rich_block_edits_match_full_ast_styles_and_source_coordinates() {
        for newline in ["\n", "\r\n"] {
            for block in [
                "Text **bold** and [link](note.md) end",
                "## 中文 *标题* 😀",
                "Text `code` $x^2$ end",
                "Text\t**bold** end",
            ] {
                let before = format!(
                    "# Before{newline}{newline}{block}{newline}{newline}> After **text**{newline}"
                );
                let snapshot = Snapshot::new(&before);
                let origin = before.find(block).unwrap();
                let mut accepted = 0;
                for (at, ch) in block.char_indices().skip(1) {
                    for replacement in [
                        "",
                        "中文",
                        "👩‍💻",
                        "**x**",
                        "[x](other.md)",
                        "\t",
                        "`x`",
                        "$y$",
                        "\\*",
                    ] {
                        let mut after = before.clone();
                        after.replace_range(origin + at..origin + at + ch.len_utf8(), replacement);
                        if let Some(updated) = snapshot.update_block(&after) {
                            accepted += 1;
                            let full = Snapshot::new(&after);
                            assert_eq!(updated.ast, full.ast, "{before:?} -> {after:?}");
                            assert_eq!(
                                crate::markdown::spans_snapshot(&updated),
                                crate::markdown::spans_snapshot(&full),
                                "{after:?}"
                            );
                            assert_eq!(updated.structural, full.structural);
                        }
                    }
                }
                assert!(
                    accepted > 20,
                    "rich edits must actually use incremental parsing"
                );
            }
        }
    }
    #[test]
    fn multiline_paragraph_edits_match_full_trees_and_styles() {
        for newline in ["\n", "\r\n", "\r"] {
            for lines in [
                [
                    "Text **bold** 中文",
                    "another [link](note.md) 😀",
                    "last *line* end",
                ],
                ["Text `multi", "line code` and \\*literal", "last $x^2$ end"],
                ["Text with hard break  ", "another line\\", "last line end"],
                [
                    "Text [multi",
                    "line](note.md) and **strong",
                    "last line** end",
                ],
            ] {
                let block = lines.join(newline);
                let before = format!(
                    "# Before{newline}{newline}{block}{newline}{newline}> After **text**{newline}{newline}Final paragraph{newline}"
                );
                let snapshot = Snapshot::new(&before);
                let origin = before.find(&block).unwrap();
                let mut accepted = 0;
                for (at, ch) in block
                    .char_indices()
                    .skip(1)
                    .filter(|(_, c)| !matches!(c, '\r' | '\n'))
                {
                    for text in [
                        "",
                        "中文",
                        "👩‍💻 e\u{301}",
                        "**x**",
                        "\t",
                        "[x](other.md)",
                        "`code`",
                        "# ",
                        "- ",
                    ] {
                        let mut after = before.clone();
                        after.replace_range(origin + at..origin + at + ch.len_utf8(), text);
                        if let Some(updated) = snapshot.update_block(&after) {
                            accepted += 1;
                            let full = Snapshot::new(&after);
                            assert_eq!(updated.ast, full.ast, "{before:?} -> {after:?}");
                            assert_eq!(
                                crate::markdown::spans_snapshot(&updated),
                                crate::markdown::spans_snapshot(&full),
                                "{after:?}"
                            );
                            assert_eq!(updated.structural, full.structural);
                        }
                    }
                }
                assert!(
                    accepted > 100,
                    "multiline edits must use the local parser: {block:?}"
                );
            }
        }
    }

    #[test]
    fn final_multiline_paragraph_updates_root_end_columns() {
        for newline in ["\n", "\r\n", "\r"] {
            let before = format!("# Before{newline}{newline}Text **bold**{newline}another line");
            let snapshot = Snapshot::new(&before);
            for text in ["中文", "\t", "👩‍💻", ""] {
                let after = before.replace("another", &format!("an{text}ther"));
                let updated = snapshot.update_block(&after).expect("local paragraph edit");
                let full = Snapshot::new(&after);
                assert_eq!(updated.ast, full.ast, "{after:?}");
                assert_eq!(
                    crate::markdown::spans_snapshot(&updated),
                    crate::markdown::spans_snapshot(&full)
                );
            }
        }
    }

    #[test]
    fn paragraph_line_break_edits_match_full_coordinates_and_styles() {
        for newline in ["\n", "\r\n", "\r"] {
            for following in [
                "",
                "\n\n# Following\n\n> quote\n\n- item\n- next\n\n```rust\nlet x = 1;\n```\n",
            ] {
                for block in [
                    "Text **bold** and 中文😀 end",
                    "Text `multi\nline code` and **bold**\nlast [link](note.md) end",
                ] {
                    let block = block.replace('\n', newline);
                    let before = format!(
                        "# Before{newline}{newline}{block}{}",
                        following.replace('\n', newline)
                    );
                    let origin = before.find(&block).unwrap();
                    let snapshot = Snapshot::new(&before);
                    let mut accepted = 0;
                    for (at, ch) in block.char_indices().skip(1) {
                        // Also split/join CRLF pairs: line deltas must come from the
                        // parsed block, not counts in the changed substring alone.
                        for inserted in [
                            "\n",
                            "\r\n",
                            "\r",
                            "\n\n",
                            "",
                            "\n中文\n",
                            "  \n",
                            "\n# heading\n",
                        ] {
                            for width in [0, ch.len_utf8()] {
                                let mut after = before.clone();
                                after.replace_range(origin + at..origin + at + width, inserted);
                                if let Some(updated) = snapshot.update_block(&after) {
                                    accepted += 1;
                                    let full = Snapshot::new(&after);
                                    assert_eq!(updated.ast, full.ast, "{before:?} -> {after:?}");
                                    assert_eq!(
                                        crate::markdown::spans_snapshot(&updated),
                                        crate::markdown::spans_snapshot(&full),
                                        "{after:?}"
                                    );
                                    assert_eq!(updated.structural, full.structural);
                                }
                            }
                        }
                    }
                    assert!(accepted > 50, "must exercise successful line edits");
                }
            }
        }
        for (before, after) in [
            ("Text **bold**", "Text\n**bold**"),
            (
                "Text **bold**\nanother line\nlast line",
                "Text **bold**another line\nlast line",
            ),
            (
                "Text **bold**\r\nanother line",
                "Text **bold**\nanother line",
            ),
        ] {
            let updated = Snapshot::new(before)
                .update_block(after)
                .expect("soft line edit must be local");
            assert_eq!(updated.ast, Snapshot::new(after).ast);
        }
    }

    #[test]
    fn multiline_structure_changes_still_fall_back() {
        for (before, after) in [
            (
                "Text **bold**\nanother line\nlast line",
                "Text **bold**\n# another line\nlast line",
            ),
            (
                "Text **bold**\nanother line\nlast line",
                "Text **bold**\n- another line\nlast line",
            ),
            (
                "Text **bold**\nanother line\nlast line",
                "Text **bold**\n\nanother line\nlast line",
            ),
            (
                "Text **bold**\nanother line\nlast line",
                "Text **bold**\nanother line\n---",
            ),
            (
                "Text [ref]\nanother line\n\n[ref]: old.md",
                "Text [ref]\nanother changed line\n\n[ref]: old.md",
            ),
            (
                "> Text **bold**\n> another line",
                "> Text **bold**\n> another changed line",
            ),
        ] {
            assert!(
                Snapshot::new(before).update_block(after).is_none(),
                "{after:?}"
            );
        }
    }

    #[test]
    fn global_and_block_structure_edits_fall_back() {
        for (before, after) in [
            ("Text **bold**", "Text %%bold%%"),
            ("Text **bold**", "Text ^[bold]"),
            ("Text [r]\n\n[r]: old.md", "Text [r] next\n\n[r]: old.md"),
            ("> Text **bold**", "> Text **boldest**"),
            ("- Text **bold**", "- Text **boldest**"),
        ] {
            assert!(
                Snapshot::new(before).update_block(after).is_none(),
                "{after}"
            );
        }
    }
}
