//! Keyboard focus visibility shared by scrollable workspace panels.
use gpui::{prelude::*, *};

/// Reveal a control when keyboard focus enters it. Remember the focused
/// descendant so ordinary redraws and manual scrolling do not pull it back.
///
/// The caller owns the vertical scroll handle and supplies a stable, window-unique
/// element ID. This wrapper adds no Tab stop and knows nothing about the owning
/// panel, workspace state, persistence, or business actions.
#[derive(IntoElement)]
pub(super) struct FocusReveal {
    id: ElementId,
    scroll: ScrollHandle,
    child: AnyElement,
}

impl FocusReveal {
    pub(super) fn new(
        id: impl Into<ElementId>,
        scroll: &ScrollHandle,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            scroll: scroll.clone(),
            child: child.into_any_element(),
        }
    }
}

impl RenderOnce for FocusReveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            (cx.focus_handle(), None::<FocusHandle>)
        });
        let focus = state.read(cx).0.clone();
        let tracked_focus = focus.clone();
        div()
            .when(focus.contains_focused(window, cx), |target| {
                target.debug_selector(|| "focus-revealed-control".into())
            })
            .on_children_prepainted(move |bounds, window, cx| {
                let current = window
                    .focused(cx)
                    .filter(|_| focus.contains_focused(window, cx));
                let changed = state.update(cx, |state, _| {
                    if state.1 == current {
                        return false;
                    }
                    state.1 = current.clone();
                    true
                });
                if !changed || current.is_none() {
                    return;
                }
                let Some(target) = bounds.first() else {
                    return;
                };
                let viewport = self.scroll.bounds();
                if viewport.size.height <= px(0.) {
                    return;
                }
                let delta = if target.top() < viewport.top() {
                    viewport.top() - target.top()
                } else if target.bottom() > viewport.bottom() {
                    viewport.bottom() - target.bottom()
                } else {
                    px(0.)
                };
                if delta != px(0.) {
                    let mut offset = self.scroll.offset();
                    offset.y += delta;
                    self.scroll.set_offset(offset);
                    window.refresh();
                }
            })
            .id(self.id)
            .flex_shrink_0()
            .min_w_0()
            .track_focus(&tracked_focus)
            .child(self.child)
    }
}
