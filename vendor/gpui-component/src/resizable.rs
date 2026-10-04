//! What this design system paints inside a resize handle.
//!
//! Base owns the band, the cursor and the drag; everything here is appearance.
//! A divider rests invisible, the way a VS Code sash does, and answers the
//! pointer with a full-length bar that thickens and takes the accent: hovered,
//! held, being dragged. There is no resting hairline and no short pill.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, ElementId, IntoElement, ParentElement as _, Pixels, Styled as _, Window,
    deferred, div, prelude::FluentBuilder as _, px,
};
use gpui_base::{
    ResizeHandleContext, ResizeHandleRenderer, ResizeHandleState, Transition, transition,
};

pub use gpui_base::{
    ResizablePanel, ResizablePanelEvent, ResizablePanelGroup, ResizableState, resizable_panel,
};

use crate::theme::ActiveTheme as _;

/// How thick the highlight is across the divider while the pointer owns it.
///
/// VS Code's sash hover bar is 4px (`--vscode-sash-hover-size`), centered on
/// the seam. That is the whole affordance: thicker than a hairline, and only
/// present while the pointer is on the handle.
const SASH_THICKNESS: Pixels = px(4.);

/// Create a [`ResizablePanelGroup`] with horizontal resizing.
pub fn h_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    gpui_base::h_resizable(id).with_handle_appearance(resize_handle_appearance())
}

/// Create a [`ResizablePanelGroup`] with vertical resizing.
pub fn v_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    gpui_base::v_resizable(id).with_handle_appearance(resize_handle_appearance())
}

/// This design system's divider appearance, for a handle that base does not
/// already hand it — a dock edge, or a hand-rolled handle in an application.
pub fn resize_handle_appearance() -> ResizeHandleRenderer {
    Rc::new(|handle, window, cx| Some(render_resize_handle(handle, window, cx)))
}

/// How thick the sash is, and how solid, at each level of engagement.
///
/// Idle draws nothing. A line on every divider would be the resting chrome
/// this appearance is replacing; the highlight is the pointer's answer.
fn sash(state: ResizeHandleState) -> (Pixels, f32) {
    match state {
        ResizeHandleState::Idle => (px(0.), 0.),
        ResizeHandleState::Hovered
        | ResizeHandleState::Pressed
        | ResizeHandleState::Dragging => (SASH_THICKNESS, 1.),
    }
}

/// The hover highlight, centered on the seam the handle resizes.
pub(crate) fn render_resize_handle(
    handle: &ResizeHandleContext,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let axis = handle.axis();
    let (target_thickness, target_opacity) = sash(handle.state());
    let motion = cx.theme().motion_tokens();
    let policy = Transition::new(motion.duration_fast).easing(motion.easing_move.clone());

    // Both values are sampled on every frame the handle is rendered, whatever
    // it is showing. A transition asks for a frame only while it is moving, so
    // a resting handle costs nothing; sampling it only while the bar is up
    // would instead leave the retained value frozen wherever it was when the
    // bar went away, and the next hover would start from there.
    let thickness = transition(
        "resizable-sash-thickness",
        target_thickness,
        policy.clone(),
        window,
        cx,
    );
    let opacity = transition("resizable-sash-opacity", target_opacity, policy, window, cx);

    div()
        // The slot fills the handle's content area exactly, so it has nothing
        // to give: shrinking it collapses the divider. The bar is thicker than
        // this slot and overhangs it.
        .flex_none()
        .flex()
        // Along the seam the bar fills the handle, so centring it there is
        // irrelevant. Across the seam it is thicker than the slot and has to
        // overhang, and neither flex alignment can be trusted to centre an
        // item that overflows -- `justify_center` returns it to the start
        // instead -- so that axis is offset by hand, below.
        .map(|slot| match axis {
            Axis::Horizontal => slot.w(px(1.)).h_full().items_center(),
            _ => slot.h(px(1.)).w_full().items_start().justify_center(),
        })
        .when(thickness > px(0.5), |slot| {
            let bar = div()
                // `flex_none` keeps the one-pixel slot from squashing it.
                .flex_none()
                .bg(cx.theme().primary)
                .opacity(opacity)
                // Half the overhang, pulled back so the bar straddles the
                // seam evenly.
                .map(|bar| match axis {
                    Axis::Horizontal => bar
                        .w(thickness)
                        .h_full()
                        .ml((thickness - px(1.)) * -0.5),
                    _ => bar
                        .h(thickness)
                        .w_full()
                        .mt((thickness - px(1.)) * -0.5),
                });
            // A hugging handle's seam is its container's outermost pixel, so
            // the bar's outer half lies past the boundary, where a dock's clip
            // would take it off. Deferring the bar -- and only the bar, only
            // while it is up -- paints it after the tree under the window's
            // mask, so it keeps that half. A straddling handle has room for
            // the overhang inside its own band, and stays in tree order: a
            // deferred element paints over the application's own deferred
            // content, and a divider that cut through a popover opened from
            // the neighbouring panel is what that looked like.
            slot.child(match handle.edge() {
                Some(_) => deferred(bar).into_any_element(),
                None => bar.into_any_element(),
            })
        })
        .into_any_element()
}
