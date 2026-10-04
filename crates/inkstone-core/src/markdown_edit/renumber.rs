use super::{Edit, prefix};
use markdown_parser::mdast::Node;

pub(super) fn removal_anchors(source: &str, edits: &[(usize, Edit)]) -> Vec<(usize, Option<u32>)> {
    let removals: Vec<_> = edits
        .iter()
        .map(|(_, edit)| edit)
        .filter(|edit| {
            !edit.range.is_empty() && edit.replacement.trim_matches([' ', '\t', '>']).is_empty()
        })
        .collect();
    if removals.is_empty() {
        return Vec::new();
    }
    fn walk(
        node: &Node,
        source: &str,
        removals: &[&Edit],
        anchors: &mut Vec<(usize, Option<u32>)>,
    ) {
        if let Node::List(list) = node
            && list.ordered
        {
            let mut first_number = None;
            let mut any_survivor = false;
            let mut previous_removed = false;
            for item in &list.children {
                let Some(position) = item.position() else {
                    continue;
                };
                let start = source[..position.start.offset]
                    .rfind('\n')
                    .map_or(0, |i| i + 1);
                let end = source[start..]
                    .find('\n')
                    .map_or(source.len(), |i| start + i);
                let Some(p) = prefix(&source[start..end]) else {
                    continue;
                };
                let Some(digits) = p.marker.strip_suffix(['.', ')']) else {
                    continue;
                };
                let Ok(number) = digits.parse::<u32>() else {
                    continue;
                };
                first_number.get_or_insert(number);
                let marker = start + p.quote.len() + p.indent.len();
                let removed = removals.iter().any(|edit| {
                    edit.range.start <= marker && marker + p.marker.len() <= edit.range.end
                });
                if !removed {
                    if previous_removed {
                        anchors.push((
                            position.start.offset,
                            (!any_survivor).then_some(first_number.unwrap()),
                        ));
                    }
                    any_survivor = true;
                }
                previous_removed = removed;
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, removals, anchors);
            }
        }
    }
    let mut anchors = Vec::new();
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    if let Ok(root) = markdown_parser::to_mdast(source, &options) {
        walk(&root, source, &removals, &mut anchors);
    }
    anchors
}

pub(super) fn at_rows(
    source: &str,
    rows: &std::collections::BTreeSet<usize>,
    starts: &std::collections::BTreeMap<usize, u32>,
) -> Vec<(std::ops::Range<usize>, String)> {
    fn number(source: &str, item: &Node) -> Option<(std::ops::Range<usize>, u32)> {
        let position = item.position()?;
        let start = source[..position.start.offset]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |i| start + i);
        let p = prefix(&source[start..end])?;
        let digits = p.marker.strip_suffix(['.', ')'])?;
        let at = start + p.quote.len() + p.indent.len();
        Some((at..at + digits.len(), digits.parse().ok()?))
    }
    fn walk(
        node: &Node,
        source: &str,
        rows: &std::collections::BTreeSet<usize>,
        starts: &std::collections::BTreeMap<usize, u32>,
        changes: &mut Vec<(std::ops::Range<usize>, String)>,
    ) {
        if let Node::List(list) = node
            && list.ordered
            && let Some(index) = list.children.iter().position(|item| {
                item.position()
                    .is_some_and(|p| rows.contains(&p.start.line))
            })
            && let Some((_, mut next)) = number(source, &list.children[index.saturating_sub(1)])
        {
            if index > 0 {
                next += 1;
            } else if let Some(value) = list.children[index]
                .position()
                .and_then(|p| starts.get(&p.start.line))
            {
                next = *value;
            }
            for item in &list.children[index..] {
                if next > 999_999_999 {
                    break;
                }
                if let Some((range, current)) = number(source, item)
                    && current != next
                {
                    changes.push((range, next.to_string()));
                }
                next += 1;
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, rows, starts, changes);
            }
        }
    }
    let mut changes = vec![];
    let mut options = markdown_parser::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    if let Ok(root) = markdown_parser::to_mdast(source, &options) {
        walk(&root, source, rows, starts, &mut changes);
    }
    changes.sort_by_key(|(range, _)| range.start);
    changes
}

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
