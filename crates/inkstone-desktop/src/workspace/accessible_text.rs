//! Non-interactive text exposed to native accessibility clients.
use gpui::{prelude::*, *};

// Painted strings alone do not expose their text to accessibility clients.
// These labels wrap without introducing keyboard tab stops.
pub(super) fn label(id: impl Into<ElementId>, value: impl Into<String>) -> Stateful<Div> {
    let value = value.into();
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(value.clone())
        .min_w_0()
        .whitespace_normal()
        .child(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn labels_expose_full_text_without_adding_focus_targets() {
        use gpui::{Element, accesskit};
        for text in [
            format!("{}/笔记👩‍💻.md", "长目录/".repeat(30)),
            "比较后本地内容已变化，请重新比较。".to_owned(),
            "正在读取磁盘版本……".to_owned(),
        ] {
            let element = label("detail", text.clone());
            assert_eq!(element.a11y_role(), Some(Role::Label));
            let mut node = accesskit::Node::new(Role::Label);
            element.write_a11y_info(&mut node);
            assert_eq!(node.label(), Some(text.as_str()));
            assert!(!node.supports_action(accesskit::Action::Focus));
            assert!(!node.supports_action(accesskit::Action::Click));
        }
    }
}
