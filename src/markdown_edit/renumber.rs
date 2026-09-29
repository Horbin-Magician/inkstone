use super::{Edit, prefix};
use markdown_parser::mdast::Node;

/// Renumber only siblings following the newly inserted ordered-list item.
/// Parsing prevents changing nested lists, code examples, or a separate list.
pub(super) fn following_items(source: &str, mut edit: Edit) -> Edit {
    let mut result = source.to_string();
    result.replace_range(edit.range.clone(), &edit.replacement);
    let row = result[..edit.selection.start]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let Ok(root) = markdown_parser::to_mdast(&result, &markdown_parser::ParseOptions::gfm()) else {
        return edit;
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
        return edit;
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
            None => current,
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
        return edit;
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
    edit
}
