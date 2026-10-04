//! What this design system paints inside a resize handle.
//!
//! Base owns the band, the cursor and the drag; everything here is appearance.
//! A divider rests as the same hairline it has always been. The pointer does
//! not grow a short pill on that line: the line itself thickens, full length,
//! and takes the accent — hovered, held, being dragged.

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

/// How thick the hairline is across the seam.
const HAIRLINE: Pixels = px(1.);

/// How thick the highlight is across the seam while the pointer owns it.
///
/// VS Code's sash hover bar is 4px (`--vscode-sash-hover-size`), centered on
/// the seam. Here that bar is the hairline grown thicker, not a pill riding
/// on it.
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

/// How thick the highlight is, and how solid, at each level of engagement.
///
/// Idle keeps the highlight off, so only the hairline shows. Hover, press and
/// drag share one bar: the whole seam, thicker, in the accent.
fn sash(state: ResizeHandleState) -> (Pixels, f32) {
    match state {
        ResizeHandleState::Idle => (HAIRLINE, 0.),
        ResizeHandleState::Hovered
        | ResizeHandleState::Pressed
        | ResizeHandleState::Dragging => (SASH_THICKNESS, 1.),
    }
}

/// The hairline, and the full-length highlight that grows out of it.
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
        // The hairline fills the handle's content area exactly, so it has
        // nothing to give: shrinking it collapses the divider.
        .flex_none()
        .flex()
        .bg(cx.theme().border)
        // Along the seam the bar fills the handle, so centring it there is
        // irrelevant. Across the seam it is thicker than the line and has to
        // overhang, and neither flex alignment can be trusted to centre an
        // item that overflows -- `justify_center` returns it to the start
        // instead -- so that axis is offset by hand, below.
        .map(|line| match axis {
            Axis::Horizontal => line.w(HAIRLINE).h_full().items_center(),
            _ => line.h(HAIRLINE).w_full().items_start().justify_center(),
        })
        .when(opacity > 0.01, |line| {
            let bar = div()
                // `flex_none` keeps the one-pixel line from squashing it.
                .flex_none()
                .bg(cx.theme().primary)
                .opacity(opacity)
                // Half the overhang, pulled back so the bar straddles the
                // hairline evenly. It grows out of the line rather than
                // appearing as a separate pill.
                .map(|bar| match axis {
                    Axis::Horizontal => bar
                        .w(thickness)
                        .h_full()
                        .ml((thickness - HAIRLINE) * -0.5),
                    _ => bar
                        .h(thickness)
                        .w_full()
                        .mt((thickness - HAIRLINE) * -0.5),
                });
            // A hugging handle's hairline is its container's outermost pixel,
            // so the bar's outer half lies past the boundary, where a dock's
            // clip would take it off. Deferring the bar -- and only the bar,
            // only while it is up -- paints it after the tree under the
            // window's mask, so it keeps that half. The hairline stays in
            // tree order: a deferred element paints over the application's own
            // deferred content, and a divider that cut through a popover
            // opened from the neighbouring panel is what that looked like.
            line.child(match handle.edge() {
                Some(_) => deferred(bar).into_any_element(),
                None => bar.into_any_element(),
            })
        })
        .into_any_element()
}
