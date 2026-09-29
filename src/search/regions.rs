use markdown_parser::mdast::Node;
use std::{cell::RefCell, ops::Range, sync::Arc};

type CachedRanges = Option<(String, Arc<Vec<Range<usize>>>)>;
pub(super) enum Kind {
    Task(Option<bool>),
    Block,
}
pub(super) struct Regions {
    kind: Kind,
    cache: RefCell<CachedRanges>,
}
impl Regions {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            cache: RefCell::new(None),
        }
    }
    pub(super) fn ranges(&self, text: &str) -> Arc<Vec<Range<usize>>> {
        if let Some((cached, ranges)) = self.cache.borrow().as_ref()
            && cached == text
        {
            return ranges.clone();
        }
        let mut options = markdown_parser::ParseOptions::gfm();
        options.constructs.frontmatter = true;
        let mut ranges = vec![];
        if let Ok(root) = markdown_parser::to_mdast(text, &options) {
            if matches!(self.kind, Kind::Block)
                && let Some(children) = root.children()
            {
                for node in children
                    .iter()
                    .filter(|node| !matches!(node, Node::List(_)))
                {
                    if let Some(position) = node.position() {
                        ranges.push(position.start.offset..position.end.offset);
                    }
                }
            }
            let mut stack = vec![&root];
            while let Some(node) = stack.pop() {
                if let Node::ListItem(item) = node
                    && let Some(position) = &item.position
                {
                    let range = position.start.offset..position.end.offset;
                    let include = match self.kind {
                        Kind::Block => true,
                        Kind::Task(required) => text
                            .get(range.clone())
                            .and_then(task_state)
                            .is_some_and(|completed| {
                                required.is_none_or(|required| required == completed)
                            }),
                    };
                    if include {
                        ranges.push(range);
                    }
                }
                if let Some(children) = node.children() {
                    stack.extend(children.iter().rev());
                }
            }
        }
        ranges.sort_by_key(|range| (range.start, range.end));
        ranges.dedup();
        let ranges = Arc::new(ranges);
        *self.cache.borrow_mut() = Some((text.to_owned(), ranges.clone()));
        ranges
    }
}
fn task_state(raw: &str) -> Option<bool> {
    let line = raw.lines().next()?.trim_start();
    let bytes = line.as_bytes();
    let marker = if matches!(bytes.first(), Some(b'-' | b'+' | b'*')) {
        1
    } else {
        let digits = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    if !bytes.get(marker)?.is_ascii_whitespace() {
        return None;
    }
    let mut body = line[marker..].trim_start().chars();
    if body.next() != Some('[') {
        return None;
    }
    let status = body.next()?;
    if body.next() != Some(']') || body.next().is_some_and(|ch| !ch.is_whitespace()) {
        return None;
    }
    Some(status != ' ')
}
