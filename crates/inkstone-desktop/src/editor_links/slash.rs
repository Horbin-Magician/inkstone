//! Block insertion through the editor's existing completion overlay.
use super::*;

const COMMANDS: &[(&str, &str, &str)] = &[
    ("一级标题", "h1 heading 标题", "# "),
    ("二级标题", "h2 heading 标题", "## "),
    ("三级标题", "h3 heading 标题", "### "),
    ("四级标题", "h4 heading 标题", "#### "),
    ("五级标题", "h5 heading 标题", "##### "),
    ("六级标题", "h6 heading 标题", "###### "),
    ("无序列表", "bullet unordered list", "- "),
    ("有序列表", "numbered ordered list", "1. "),
    ("任务列表", "task todo checklist", "- [ ] "),
    ("表格", "table", "| 列 1 | 列 2 |\n| --- | --- |\n|  |  |"),
    ("Callout 提示块", "callout note 提示", "> [!note]\n> "),
    ("引用", "quote", "> "),
    ("代码块", "code fence", "```\n\n```"),
    ("分隔线", "divider horizontal rule", "---\n\n"),
];

/// Only a slash at the start of a line is a block command. This inexpensive
/// check is also used when requesting fresh results after committed input.
pub(crate) fn query(source: &str, offset: usize) -> Option<(usize, &str)> {
    let before = source.get(..offset)?;
    let start = before.rfind(['\r', '\n']).map_or(0, |at| at + 1);
    let typed = before[start..].strip_prefix('/')?;
    (!typed.contains(['/', '\\', '\r', '\n', '[', ']'])).then_some((start, typed))
}

pub(super) fn completions(
    source: &str,
    text: &Rope,
    offset: usize,
) -> Option<Vec<lsp_types::CompletionItem>> {
    let (start, typed) = query(source, offset)?;
    if !accepts_wiki_completion(source, start) {
        return Some(vec![]);
    }
    let query = typed.trim().to_lowercase();
    Some(
        COMMANDS
            .iter()
            .filter(|(label, aliases, _)| {
                label.to_lowercase().contains(&query) || aliases.contains(&query)
            })
            .map(|(label, _, replacement)| lsp_types::CompletionItem {
                label: (*label).into(),
                label_details: Some(lsp_types::CompletionItemLabelDetails {
                    detail: None,
                    description: Some("插入 Markdown 块".into()),
                }),
                text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                    range: lsp_types::Range::new(
                        text.offset_to_position(start),
                        text.offset_to_position(offset),
                    ),
                    new_text: (*replacement).into(),
                })),
                ..Default::default()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_commands_recognize_all_markdown_line_endings() {
        for newline in ["\n", "\r\n", "\r"] {
            let prefix = format!("中文😀{newline}{newline}");
            let source = format!("{prefix}/h2保留");
            let offset = prefix.len() + 3;
            assert_eq!(query(&source, offset), Some((prefix.len(), "h2")));
            let rope = Rope::from(source.clone());
            let items = completions(&source, &rope, offset).unwrap();
            assert_eq!(items.len(), 1);
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
                panic!("edit expected");
            };
            let mut result = source.clone();
            result.replace_range(
                rope.position_to_offset(&edit.range.start)
                    ..rope.position_to_offset(&edit.range.end),
                &edit.new_text,
            );
            assert_eq!(result, format!("{prefix}## 保留"));
        }
    }

    #[test]
    fn slash_context_excludes_paths_and_literal_blocks() {
        for source in [
            "text /", "https://", "[[/", "\\/", "    /", "```md\n/", "~~~\n/", "<!--\n/",
        ] {
            let items = completions(source, &Rope::from(source), source.len());
            assert!(items.is_none_or(|items| items.is_empty()), "{source}");
        }
    }

    #[test]
    fn slash_filters_and_replaces_only_the_command_in_unicode_text() {
        for (query, label, replacement) in [
            ("H2", "二级标题", "## "),
            ("任务", "任务列表", "- [ ] "),
            ("callout", "Callout 提示块", "> [!note]\n> "),
            ("table", "表格", "| 列 1 | 列 2 |\n| --- | --- |\n|  |  |"),
        ] {
            let prefix = "中文😀\n\n";
            let source = format!("{prefix}/{query}保留");
            let offset = prefix.len() + 1 + query.len();
            let rope = Rope::from(source.clone());
            let items = completions(&source, &rope, offset).unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].label, label);
            let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
                panic!("edit expected")
            };
            let mut result = source.clone();
            result.replace_range(
                rope.position_to_offset(&edit.range.start)
                    ..rope.position_to_offset(&edit.range.end),
                &edit.new_text,
            );
            assert_eq!(result, format!("{prefix}{replacement}保留"));
        }
        assert_eq!(
            completions("/", &Rope::from("/"), 1).unwrap().len(),
            COMMANDS.len()
        );
        assert!(
            completions("/unknown", &Rope::from("/unknown"), 8)
                .unwrap()
                .is_empty()
        );
    }
}
