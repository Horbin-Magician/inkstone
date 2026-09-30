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
        let comments = crate::comments::ranges(text);
        let clean = crate::comments::masked(text, &comments);
        if let Ok(root) = markdown_parser::to_mdast(&clean, &options) {
            if matches!(self.kind, Kind::Block)
                && let Some(children) = root.children()
            {
                for node in children
                    .iter()
                    .filter(|node| !matches!(node, Node::List(_)))
                {
                    if let Some(position) = node.position() {
                        let range = position.start.offset..position.end.offset;
                        if clean[range.clone()]
                            .chars()
                            .any(|ch| ch != '\u{1}' && !ch.is_whitespace())
                        {
                            ranges.push(range);
                        }
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
    crate::index::task_marker(raw).map(|(_, checked)| checked)
}
