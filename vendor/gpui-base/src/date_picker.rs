use crate::TestSupportExt as _;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, FocusHandle, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, Role, StatefulInteractiveElement, StyleRefinement, Styled, Window,
    div,
};

use crate::{
    StyledExt as _,
    actions::{Cancel, Confirm},
};

type OpenChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// Unstyled controlled date-picker root. Applications own its trigger, calendar,
/// positioning, and visual presentation; Base owns focus and open/dismiss keyboard behavior.
#[derive(IntoElement)]
pub struct DatePicker {
    base: crate::ObservedElement<gpui::Stateful<Div>>,
    open: bool,
    disabled: bool,
    focus_handle: FocusHandle,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    on_open_change: Option<OpenChange>,
}

impl DatePicker {
    pub fn new(id: impl Into<ElementId>, focus_handle: &FocusHandle) -> Self {
        Self {
            base: div().id(id).test_support(),
            open: false,
            disabled: false,
            focus_handle: focus_handle.clone(),
            style: StyleRefinement::default(),
            children: vec![],
            on_open_change: None,
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn on_open_change(
        mut self,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }
}
impl Styled for DatePicker {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl ParentElement for DatePicker {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}
impl InteractiveElement for DatePicker {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.focus_handle = handle.clone();
        self
    }
}
impl StatefulInteractiveElement for DatePicker {}
impl RenderOnce for DatePicker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let open = self.open;
        let disabled = self.disabled;
        let handler = self.on_open_change;
        let base = self
            .base
            .role(Role::ComboBox)
            .aria_expanded(open)
            .track_focus(&self.focus_handle.tab_stop(!disabled))
            .on_action({
                let handler = handler.clone();
                move |_: &Confirm, window, cx| {
                    // Enter opens the picker, and closes it again once the
                    // value shown in the popup is the one the user wants.
                    if disabled {
                        cx.propagate();
                    } else if let Some(handler) = &handler {
                        handler(!open, window, cx);
                    }
                }
            })
            .on_action(move |_: &Cancel, window, cx| {
                if open {
                    if let Some(handler) = &handler {
                        handler(false, window, cx);
                    }
                } else {
                    cx.propagate();
                }
            })
            .refine_style(&self.style)
            .children(self.children);
        crate::accessibility::with_disabled(base, disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, px};

    struct FocusProbe {
        original: FocusHandle,
        replacement: FocusHandle,
    }

    impl Render for FocusProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            DatePicker::new("picker", &self.original)
                .track_focus(&self.replacement)
                .size(px(100.))
        }
    }

    #[gpui::test]
    fn explicit_focus_binding_replaces_constructor_handle(cx: &mut TestAppContext) {
        let (probe, cx) = cx.add_window_view(|_, cx| FocusProbe {
            original: cx.focus_handle(),
            replacement: cx.focus_handle(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
            assert!(probe.read(cx).replacement.is_focused(window));
            assert!(!probe.read(cx).original.is_focused(window));
        });
    }

    #[gpui::test]
    fn native_semantics_report_disabled_and_expanded_state(cx: &mut TestAppContext) {
        use gpui::{Element as _, accesskit};
        let (_, cx) = cx.add_window_view(|_, cx| FocusProbe {
            original: cx.focus_handle(),
            replacement: cx.focus_handle(),
        });
        cx.update(|window, cx| {
            for disabled in [false, true] {
                for open in [false, true] {
                    let focus = cx.focus_handle();
                    let element = DatePicker::new("semantics", &focus)
                        .disabled(disabled)
                        .open(open)
                        .render(window, cx)
                        .into_element();
                    assert_eq!(element.a11y_role(), Some(Role::ComboBox));
                    let mut node = accesskit::Node::new(element.a11y_role().unwrap());
                    element.write_a11y_info(&mut node);
                    assert_eq!(node.is_expanded(), Some(open));
                    assert_eq!(node.is_disabled(), disabled);
                }
            }
        });
    }

    struct ActionProbe {
        focus: FocusHandle,
        open: bool,
        disabled: bool,
        changes: Rc<std::cell::RefCell<Vec<bool>>>,
    }

    impl Render for ActionProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let changes = self.changes.clone();
            DatePicker::new("action-picker", &self.focus)
                .open(self.open)
                .disabled(self.disabled)
                .on_open_change(move |open, _, _| changes.borrow_mut().push(open))
                .size(px(100.))
        }
    }

    #[gpui::test]
    fn confirm_respects_disabled_while_cancel_can_dismiss_an_open_picker(cx: &mut TestAppContext) {
        for (open, disabled, expected) in [
            (false, false, vec![true]),
            (true, false, vec![false, false]),
            (false, true, vec![]),
            (true, true, vec![false]),
        ] {
            let changes = Rc::new(std::cell::RefCell::new(Vec::new()));
            let (probe, cx) = cx.add_window_view({
                let changes = changes.clone();
                move |_, cx| ActionProbe {
                    focus: cx.focus_handle(),
                    open,
                    disabled,
                    changes,
                }
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let focus = probe.read(cx).focus.clone();
                focus.focus(window, cx);
                window.draw(cx).clear(cx);
            });
            cx.dispatch_action(Confirm { secondary: false });
            cx.dispatch_action(Cancel);
            assert_eq!(*changes.borrow(), expected);
        }
    }
}
