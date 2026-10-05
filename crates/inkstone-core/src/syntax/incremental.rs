//! Reparse independent top-level inline blocks; structural/global edits fall back.
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
        if old[start..old_end].contains(['\r', '\n'])
            || source[start..new_end].contains(['\r', '\n'])
        {
            return None;
        }
        let ast = self.ast.as_deref()?;
        if has_definitions(ast) {
            return None;
        }
        let Node::Root(root) = ast else {
            return None;
        };
        let (index, block) = root.children.iter().enumerate().find(|(_, node)| {
            matches!(node, Node::Paragraph(_) | Node::Heading(_))
                && node.position().is_some_and(|p| {
                    p.start.column == 1
                        && p.start.line == p.end.line
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
        let mut ast = ast.clone();
        ast.children_mut()?.remove(index);
        shift(&mut ast, old_end, delta, position.end.line, source);
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
fn shift(node: &mut Node, at: usize, delta: isize, line: usize, source: &str) {
    if let Some(p) = node.position_mut() {
        for point in [&mut p.start, &mut p.end] {
            if point.offset >= at {
                point.offset = point.offset.checked_add_signed(delta).unwrap();
            }
            if point.line == line {
                let start = source[..point.offset].rfind('\n').map_or(0, |i| i + 1);
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
            shift(child, at, delta, line, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn global_and_block_structure_edits_fall_back() {
        for (before, after) in [
            ("Text **bold**", "Text\n**bold**"),
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
