//! One source-coordinate parse shared by indexing and live presentation.
use markdown_parser::{ParseOptions, mdast::Node};
use std::sync::Arc;

pub fn options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

#[derive(Clone)]
pub struct Snapshot {
    pub source: Arc<str>,
    pub structural: Arc<str>,
    pub comments: Vec<crate::comments::Comment>,
    pub ast: Option<Arc<Node>>,
}

impl Snapshot {
    pub fn new(source: &str) -> Self {
        let comments = crate::comments::ranges(source);
        let structural: Arc<str> = crate::comments::masked(source, &comments).as_ref().into();
        let ast = markdown_parser::to_mdast(&structural, &options())
            .ok()
            .map(Arc::new);
        Self {
            source: source.into(),
            structural,
            comments,
            ast,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn math_and_code_do_not_leak_note_structure_or_comment_markers() {
        let source = "# 真标题\r\n$\\text{[[伪链接]] #伪标签 %%公式%% ==文字==}$\r\n$$\r\n# 伪标题\r\n- [ ] 伪任务\r\n[[伪链接]]\r\n$$\r\n```mermaid\r\nA[\"[[伪链接]]\"]\r\n```\r\n[[真实]] %%注释%%";
        let snapshot = Snapshot::new(source);
        assert_eq!(snapshot.structural.len(), source.len());
        assert_eq!(snapshot.comments.len(), 1);
        let parsed = crate::index::parse_snapshot(&snapshot);
        assert_eq!(parsed.headings.len(), 1);
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.links[0].target, "真实");
        assert!(parsed.tasks.is_empty());
        assert!(parsed.tags.is_empty());
        let styles = crate::markdown::spans_snapshot(&snapshot);
        assert!(
            !styles
                .iter()
                .any(|s| s.kind == crate::markdown::Kind::Highlight)
        );
    }
}
