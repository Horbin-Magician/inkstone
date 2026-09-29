use markdown_parser::mdast::Node;
use std::{cell::RefCell, ops::Range, sync::Arc};

pub(super) struct Section {
    pub range: Range<usize>,
    pub descendants_end: usize,
}
type Cache = Option<(String, Arc<Vec<Section>>)>;
#[derive(Default)]
pub(super) struct Sections {
    cache: RefCell<Cache>,
}
impl Sections {
    fn get(&self, text: &str) -> Arc<Vec<Section>> {
        if let Some((source, sections)) = self.cache.borrow().as_ref()
            && source == text
        {
            return sections.clone();
        }
        let mut options = markdown_parser::ParseOptions::gfm();
        options.constructs.frontmatter = true;
        let mut headings = vec![];
        if let Ok(root) = markdown_parser::to_mdast(text, &options) {
            let mut stack = vec![&root];
            while let Some(node) = stack.pop() {
                if let Node::Heading(heading) = node
                    && let Some(position) = &heading.position
                {
                    headings.push((position.start.offset, heading.depth));
                }
                if let Some(children) = node.children() {
                    stack.extend(children.iter().rev());
                }
            }
        }
        headings.sort_unstable();
        let mut sections: Vec<Section> = vec![];
        if headings.is_empty() {
            sections.push(Section {
                range: 0..text.len(),
                descendants_end: 1,
            });
        } else if headings[0].0 > 0 {
            sections.push(Section {
                range: 0..headings[0].0,
                descendants_end: 1,
            });
        }
        let mut ancestors: Vec<(usize, u8)> = vec![];
        for (i, &(start, level)) in headings.iter().enumerate() {
            let index = sections.len();
            while ancestors.last().is_some_and(|(_, old)| *old >= level) {
                let (ancestor, _) = ancestors.pop().unwrap();
                sections[ancestor].descendants_end = index;
            }
            sections.push(Section {
                range: start..headings.get(i + 1).map_or(text.len(), |h| h.0),
                descendants_end: index + 1,
            });
            ancestors.push((index, level));
        }
        for (index, _) in ancestors {
            sections[index].descendants_end = sections.len();
        }
        let sections = Arc::new(sections);
        *self.cache.borrow_mut() = Some((text.into(), sections.clone()));
        sections
    }
}
pub(super) struct Context<'a> {
    pub source: &'a str,
    pub sections: Arc<Vec<Section>>,
    pub index: usize,
}
pub(super) struct View<'a> {
    pub source: &'a str,
    pub sections: Arc<Vec<Section>>,
    pub candidates: Range<usize>,
    pub base: usize,
}
impl<'a> View<'a> {
    pub fn new(cache: &Sections, text: &'a str, context: Option<&Context<'a>>) -> Self {
        if let Some(context) = context {
            Self {
                source: context.source,
                sections: context.sections.clone(),
                candidates: context.index + 1..context.sections[context.index].descendants_end,
                base: context.sections[context.index].range.start,
            }
        } else {
            let sections = cache.get(text);
            Self {
                candidates: 0..sections.len(),
                source: text,
                sections,
                base: 0,
            }
        }
    }
    pub fn context(&self, index: usize) -> Context<'a> {
        Context {
            source: self.source,
            sections: self.sections.clone(),
            index,
        }
    }
}
