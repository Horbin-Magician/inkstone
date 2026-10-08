//! Shared code-block chrome for reading and live preview.
use gpui::{prelude::*, *};
use gpui_base::text::CodeBlock;
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
};

pub(super) fn style(font_size: f32) -> StyleRefinement {
    StyleRefinement::default()
        .text_size(px(font_size * 0.875))
        .pt(px(40.))
}

pub(super) fn actions(block: &CodeBlock, _: &mut Window, cx: &mut App) -> Div {
    let code = block.code();
    let light = !gpui_component::Theme::global(cx).is_dark();
    div()
        .flex()
        .items_center()
        .gap_2()
        .font_family(gpui_component::Theme::global(cx).font_family.clone())
        .text_size(px(crate::theme::MIN_UI_FONT_SIZE))
        .text_color(crate::theme::palette(light).muted)
        .child(
            div()
                .max_w(px(120.))
                .truncate()
                .child(block.lang().unwrap_or_else(|| "text".into())),
        )
        .child(
            div()
                .id("copy-code-control")
                .debug_selector(|| "copy-code-control".into())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new("copy-code")
                        .small()
                        .ghost()
                        .label("复制")
                        .tooltip("复制代码")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            window.prevent_default();
                            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_component::highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter};

    #[gpui::test]
    fn code_highlighting_is_installed_and_supports_common_languages(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            assert!(gpui_base::text::TextViewDefaults::global(cx).has_code_block_highlighter())
        });
        for (language, source) in [
            ("rust", "fn main() { let name = \"中文😀\"; }"),
            ("python", "def hello():\n    return '中文😀'"),
            ("js", "const name = '中文😀';"),
            ("ts", "const count: number = 42;"),
            ("tsx", "const view = <div>Hello</div>;"),
            ("json", "{\"name\": \"中文😀\"}"),
            ("bash", "echo \"hello\""),
            ("yaml", "name: hello"),
            ("toml", "name = \"hello\""),
            ("html", "<div>hello</div>"),
            ("css", "body { color: red; }"),
            ("sql", "SELECT name FROM notes;"),
            ("c", "int main(void) { return 0; }"),
            ("cpp", "class Note {};"),
            ("java", "class Note { int count = 1; }"),
            ("go", "package main\nfunc main() {}"),
            ("md", "# Title\n\n**bold**"),
        ] {
            assert!(
                LanguageRegistry::singleton().language(language).is_some(),
                "{language}"
            );
            let mut highlighter = SyntaxHighlighter::new(language);
            highlighter.update(None, &ropey::Rope::from_str(source), None);
            for theme in [
                HighlightTheme::default_dark(),
                HighlightTheme::default_light(),
            ] {
                let styles = highlighter.styles(&(0..source.len()), theme.as_ref());
                assert!(
                    styles.iter().any(|(_, style)| style.color.is_some()),
                    "{language}"
                );
                for (range, _) in styles {
                    assert!(
                        source.get(range).is_some(),
                        "{language}: invalid UTF-8 range"
                    );
                }
            }
        }
        let mut plain = SyntaxHighlighter::new("unknown-language");
        plain.update(None, &ropey::Rope::from_str("中文😀"), None);
        assert!(plain.tree().is_none());
        assert_eq!(
            plain.styles(&(0..10), HighlightTheme::default_dark().as_ref()),
            vec![(0..10, HighlightStyle::default())]
        );
    }
}
