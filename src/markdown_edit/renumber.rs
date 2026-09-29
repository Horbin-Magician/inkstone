use super::{Edit, prefix};
use markdown_parser::mdast::Node;

/// Renumber only siblings following the newly inserted ordered-list item.
/// Parsing prevents changing nested lists, code examples, or a separate list.
pub(super) fn following_items(source: &str, mut edit: Edit) -> Edit {
    renumber_from(source, &mut edit, None);
    edit
}

pub(super) fn removed_item(source: &str, mut edit: Edit) -> Edit {
    let row = source[..edit.range.start]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let Ok(root) = markdown_parser::to_mdast(source, &markdown_parser::ParseOptions::gfm()) else {
        return edit;
    };
    fn next_item(node: &Node, row: usize, source: &str) -> Option<(usize, u32)> {
        if let Node::List(list) = node
            && list.ordered
            && let Some(index) = list
                .children
                .iter()
                .position(|item| item.position().is_some_and(|p| p.start.line == row))
        {
            let next_row = list.children.get(index + 1)?.position()?.start.line;
            let baseline = list.children[index.saturating_sub(1)].position()?;
            let line_start = source[..baseline.start.offset]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            let line_end = source[line_start..]
                .find('\n')
                .map_or(source.len(), |i| line_start + i);
            let p = prefix(&source[line_start..line_end])?;
            let number = p.marker.strip_suffix(['.', ')'])?.parse::<u32>().ok()?;
            return Some((next_row, number + u32::from(index > 0)));
        }
        node.children()?
            .iter()
            .find_map(|child| next_item(child, row, source))
    }
    if let Some(target) = next_item(&root, row, source) {
        renumber_from(source, &mut edit, Some(target));
    }
    edit
}

fn renumber_from(source: &str, edit: &mut Edit, target: Option<(usize, u32)>) {
    let mut result = source.to_string();
    result.replace_range(edit.range.clone(), &edit.replacement);
    let row = target.map_or_else(
        || {
            result[..edit.selection.start]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1
        },
        |(row, _)| row,
    );
    let Ok(root) = markdown_parser::to_mdast(&result, &markdown_parser::ParseOptions::gfm()) else {
        return;
    };
    fn siblings(node: &Node, row: usize) -> Option<&[Node]> {
        if let Node::List(list) = node
            && list.ordered
            && let Some(index) = list
                .children
                .iter()
                .position(|item| item.position().is_some_and(|p| p.start.line == row))
        {
            return Some(&list.children[index..]);
        }
        node.children()?
            .iter()
            .find_map(|child| siblings(child, row))
    }
    let Some(items) = siblings(&root, row) else {
        return;
    };
    let mut number = None;
    let mut changes = Vec::new();
    for item in items {
        let Some(position) = item.position() else {
            break;
        };
        let start = result[..position.start.offset]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let end = result[start..]
            .find('\n')
            .map_or(result.len(), |i| start + i);
        let Some(p) = prefix(&result[start..end]) else {
            break;
        };
        let Some(digits) = p.marker.strip_suffix(['.', ')']) else {
            break;
        };
        let Ok(current) = digits.parse::<u32>() else {
            break;
        };
        let next = match number {
            None => target.map_or(current, |(_, number)| number),
            Some(previous) => previous + 1,
        };
        if next > 999_999_999 {
            break;
        }
        number = Some(next);
        if next != current {
            let at = start + p.quote.len() + p.indent.len();
            changes.push((at..at + digits.len(), next.to_string()));
        }
    }
    if changes.is_empty() {
        return;
    }
    for (range, replacement) in changes.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    // Keep this one source transaction, trimming the unaffected suffix on character boundaries.
    let suffix = source[edit.range.end..]
        .chars()
        .rev()
        .zip(result[edit.selection.end..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    edit.range.end = source.len() - suffix;
    edit.replacement = result[edit.range.start..result.len() - suffix].to_string();
}
