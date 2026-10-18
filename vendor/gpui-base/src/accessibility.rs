//! Native accessibility properties not exposed by GPUI's Div builders.
use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hitbox, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, Pixels, Window,
};

/// Keep observation outside the native property adapter so tests see exactly
/// the properties written into the platform accessibility node.
pub(crate) fn with_disabled<E>(
    element: crate::ObservedElement<E>,
    disabled: bool,
) -> crate::ObservedElement<AccessibilityElement<E>>
where
    E: Element<PrepaintState = Option<Hitbox>> + InteractiveElement,
{
    with_control_state(element, disabled, false)
}

pub(crate) fn with_control_state<E>(
    element: crate::ObservedElement<E>,
    disabled: bool,
    busy: bool,
) -> crate::ObservedElement<AccessibilityElement<E>>
where
    E: Element<PrepaintState = Option<Hitbox>> + InteractiveElement,
{
    #[cfg(feature = "test-support")]
    {
        element.map_inner(|inner| AccessibilityElement {
            inner,
            disabled,
            busy,
        })
    }
    #[cfg(not(feature = "test-support"))]
    {
        AccessibilityElement {
            inner: element,
            disabled,
            busy,
        }
    }
}

/// No extra layout, hitbox, identity, focus binding or accessibility node.
pub(crate) struct AccessibilityElement<E> {
    inner: E,
    disabled: bool,
    busy: bool,
}

impl<E: Element> IntoElement for AccessibilityElement<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<E: Element + InteractiveElement> InteractiveElement for AccessibilityElement<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl<E: Element> Element for AccessibilityElement<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.inner
            .prepaint(id, inspector, bounds, layout, window, cx)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        paint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector, bounds, layout, paint, window, cx);
    }
    fn a11y_role(&self) -> Option<gpui::Role> {
        self.inner.a11y_role()
    }
    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        self.inner.write_a11y_info(node);
        if self.disabled {
            node.set_disabled();
        }
        if self.busy {
            node.set_busy();
            node.remove_action(gpui::accesskit::Action::Click);
        }
    }
    fn a11y_synthetic_children(
        &mut self,
        paint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        self.inner.a11y_synthetic_children(paint, builder);
    }
}
