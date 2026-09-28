//! Public editor extension interfaces only; no component patches.
use gpui::{App, SharedString, Task, WeakEntity, Window};
use gpui_component::input::{CompletionProvider, DefinitionProvider, EditorState, Rope, RopeExt};
use inkstone::index::ParsedNote;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub type ParsedCache = Rc<RefCell<(SharedString, ParsedNote)>>;
pub type PathCache = Rc<RefCell<Arc<Vec<String>>>>;
pub struct WikiDefinitions(pub ParsedCache);
pub struct WikiCompletions(pub PathCache, pub Option<WeakEntity<EditorState>>);

pub fn target_uri(target: &str) -> lsp_types::Uri {
    let hex: String = target
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("inkstone:{hex}").parse().expect("hex URI")
}
pub fn decode_uri(uri: &str) -> Option<String> {
    let hex = uri.strip_prefix("inkstone:")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<_>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}
impl DefinitionProvider for WikiDefinitions {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<lsp_types::LocationLink>>> {
        let cache = self.0.borrow();
        if cache.0.len() != text.len() || cache.0.as_ref() != text {
            return Task::ready(Ok(vec![]));
        }
        let result = cache
            .1
            .links
            .iter()
            .find(|link| link.range.contains(&offset))
            .map(|link| lsp_types::LocationLink {
                origin_selection_range: Some(lsp_types::Range::new(
                    text.offset_to_position(link.range.start),
                    text.offset_to_position(link.range.end),
                )),
                target_uri: target_uri(&link.target),
                target_range: Default::default(),
                target_selection_range: Default::default(),
            })
            .into_iter()
            .collect();
        Task::ready(Ok(result))
    }
}
impl CompletionProvider for WikiCompletions {
    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty() && !new_text.contains(['\n', '\r', ']'])
    }
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: lsp_types::CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<lsp_types::CompletionResponse>> {
        let text = text.clone();
        let snapshot = text.clone();
        let editor = self.1.clone();
        let paths = self.0.borrow().clone();
        let task = cx.background_executor().spawn(async move {
            let source = text.to_string();
            let Some(before) = source.get(..offset) else {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            };
            let Some(start) = before.rfind("[[") else {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            };
            let typed = &before[start + 2..];
            if typed.contains([']', '\n', '\r', '|']) {
                return Ok(lsp_types::CompletionResponse::Array(vec![]));
            }
            let end = if source[offset..].starts_with("]]") {
                offset + 2
            } else {
                offset
            };
            let query = typed.to_lowercase();
            let range = lsp_types::Range::new(
                text.offset_to_position(start + 2),
                text.offset_to_position(end),
            );
            let items = paths
                .iter()
                .filter(|path| path.to_lowercase().contains(&query))
                .take(40)
                .map(|path| lsp_types::CompletionItem {
                    label: path.clone(),
                    text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                        range,
                        new_text: format!("{path}]]"),
                    })),
                    ..Default::default()
                })
                .collect();
            Ok(lsp_types::CompletionResponse::Array(items))
        });
        cx.spawn(async move |cx| {
            let response = task.await;
            if let Some(editor) = editor {
                let valid = editor
                    .read_with(cx, |state, _| {
                        state.cursor() == offset && state.text() == &snapshot
                    })
                    .unwrap_or(false);
                if !valid {
                    return Ok(lsp_types::CompletionResponse::Array(vec![]));
                }
            }
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_uri_roundtrip_chinese_and_spaces() {
        let target = "目录/中文 空格#标题";
        assert_eq!(
            decode_uri(&target_uri(target).to_string()).as_deref(),
            Some(target)
        );
        assert_eq!(decode_uri("inkstone:xyz"), None);
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    async fn stale_completion_result_is_discarded(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("[[笔", window, cx));
        let task = handle
            .update(cx, |pane, window, cx| {
                let editor = pane.editor.clone();
                editor.update(cx, |state, cx| state.set_selected_range(5..5, cx));
                let provider = WikiCompletions(
                    Rc::new(RefCell::new(Arc::new(vec!["/笔记".into()]))),
                    Some(editor.downgrade()),
                );
                let task = provider.completions(
                    &Rope::from("[[笔"),
                    5,
                    lsp_types::CompletionContext {
                        trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                        trigger_character: None,
                    },
                    window,
                    cx,
                );
                editor.update(cx, |state, cx| state.set_value("另一个文档", window, cx));
                task
            })
            .unwrap();
        let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
            panic!("array expected")
        };
        assert!(items.is_empty());
    }

    #[gpui::test]
    async fn completions_and_definitions_use_component_unicode_coordinates(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| crate::editor::EditorPane::new("", window, cx));
        let text = Rope::from("中😀 [[笔");
        let provider = WikiCompletions(
            Rc::new(RefCell::new(Arc::new(vec![
                "/目录/笔记".into(),
                "/其他".into(),
            ]))),
            None,
        );
        let task = handle
            .update(cx, |_, window, cx| {
                provider.completions(
                    &text,
                    text.len(),
                    lsp_types::CompletionContext {
                        trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                        trigger_character: None,
                    },
                    window,
                    cx,
                )
            })
            .unwrap();
        let lsp_types::CompletionResponse::Array(items) = task.await.unwrap() else {
            panic!("array expected")
        };
        assert_eq!(items.len(), 1);
        let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
            panic!("edit expected")
        };
        assert_eq!(text.position_to_offset(&edit.range.start), "中😀 [[".len());
        assert_eq!(edit.new_text, "/目录/笔记]]");
        let source = "中😀 [[目录/笔记|标签]]";
        let provider = WikiDefinitions(Rc::new(RefCell::new((
            source.into(),
            inkstone::index::parse(source),
        ))));
        let text = Rope::from(source);
        let task = handle
            .update(cx, |_, window, cx| {
                provider.definitions(&text, "中😀 [[".len(), window, cx)
            })
            .unwrap();
        let links = task.await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            decode_uri(&links[0].target_uri.to_string()).unwrap(),
            "目录/笔记"
        );
    }
}
