//! Markdown image syntax targeting local Markdown notes.
use markdown_parser::mdast::Node;
use std::{collections::BTreeMap, ops::Range, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarkdownEmbed {
    pub range: Range<usize>,
    pub target: String,
    pub label: String,
}

fn note_target(url: &str) -> bool {
    let path = url.split('#').next().unwrap_or("");
    let Ok(path) = percent_encoding::percent_decode_str(path).decode_utf8() else {
        return false;
    };
    !path.contains(':')
        && !path.starts_with("//")
        && Path::new(path.as_ref())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

pub(crate) fn scan(ast: &Node) -> Vec<MarkdownEmbed> {
    fn definitions<'a>(node: &'a Node, out: &mut BTreeMap<String, &'a str>) {
        if let Node::Definition(n) = node {
            out.entry(n.identifier.to_lowercase()).or_insert(&n.url);
        }
        if let Some(children) = node.children() {
            for child in children {
                definitions(child, out);
            }
        }
    }
    fn images(node: &Node, definitions: &BTreeMap<String, &str>, out: &mut Vec<MarkdownEmbed>) {
        let image = match node {
            Node::Image(n) => Some((n.url.as_str(), &n.alt, &n.position)),
            Node::ImageReference(n) => definitions
                .get(&n.identifier.to_lowercase())
                .map(|url| (*url, &n.alt, &n.position)),
            _ => None,
        };
        if let Some((url, alt, Some(p))) = image
            && note_target(url)
        {
            out.push(MarkdownEmbed {
                range: p.start.offset..p.end.offset,
                target: url.into(),
                label: alt.clone(),
            });
        }
        if let Some(children) = node.children() {
            for child in children {
                images(child, definitions, out);
            }
        }
    }
    let mut definitions_map = BTreeMap::new();
    definitions(ast, &mut definitions_map);
    let mut embeds = vec![];
    images(ast, &definitions_map, &mut embeds);
    embeds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_note_images_are_embeds() {
        let source = "![remote](https://example.com/note.md)\n![remote](//example.com/note.md)\n![scheme](file:///note.md)\n![invalid](%FF.md)\n![image](image.png)\n`![code](note.md)`\n\\![escaped](note.md)";
        let snapshot = crate::syntax::Snapshot::new(source);
        assert!(scan(snapshot.ast.as_deref().unwrap()).is_empty());
        let source = "![local](/folder/note.MD#section)\n![missing][missing]";
        let snapshot = crate::syntax::Snapshot::new(source);
        let embeds = scan(snapshot.ast.as_deref().unwrap());
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].target, "/folder/note.MD#section");
    }
}
