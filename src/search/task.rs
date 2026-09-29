use markdown_parser::mdast::Node;
use std::{cell::RefCell, ops::Range, sync::Arc};

type CachedRanges = Option<(String, Arc<Vec<Range<usize>>>)>;
pub(super) struct Tasks {
    completed: Option<bool>,
    cache: RefCell<CachedRanges>,
}
impl Tasks {
    pub(super) fn new(completed: Option<bool>) -> Self {
        Self {
            completed,
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
            let mut stack = vec![&root];
            while let Some(node) = stack.pop() {
                if let Node::ListItem(item) = node
                    && let Some(position) = &item.position
                {
                    let range = position.start.offset..position.end.offset;
                    if let Some(raw) = text.get(range.clone())
                        && let Some(completed) = task_state(raw)
                        && self.completed.is_none_or(|required| required == completed)
                    {
                        ranges.push(range);
                    }
                }
                if let Some(children) = node.children() {
                    stack.extend(children.iter().rev());
                }
            }
        }
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
