use super::*;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct WheelZoom {
    last_scroll: Option<Instant>,
}

impl WheelZoom {
    fn delta(&mut self, enabled: bool, control: bool, y: f32, now: Instant) -> Option<i8> {
        if !enabled {
            self.last_scroll = None;
            return None;
        }
        if !control
            || self
                .last_scroll
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500))
        {
            self.last_scroll = Some(now);
            return None;
        }
        if y == 0. || !y.is_finite() {
            None
        } else {
            Some(if y > 0. { 1 } else { -1 })
        }
    }
}

pub(super) fn capture(pane: WeakEntity<EditorPane>) -> impl IntoElement {
    // Capture before the input's scroll handler consumes the event. A normal
    // hitbox permits editing underneath; modal occlusion still excludes it.
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let _ = pane.update(cx, |pane, cx| {
                    if let Some(delta) = pane.font_zoom.delta(
                        pane.quick_font_size,
                        event.modifiers.control,
                        f32::from(event.delta.pixel_delta(px(20.)).y),
                        Instant::now(),
                    ) {
                        cx.emit(EditorEvent::FontSizeDelta(delta));
                    }
                });
            });
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn wheel_steps_and_scroll_cooldown() {
        let mut zoom = WheelZoom::default();
        let now = Instant::now();
        assert_eq!(zoom.delta(false, true, 80., now), None);
        assert_eq!(zoom.delta(true, true, 80., now), Some(1));
        assert_eq!(zoom.delta(true, true, -0.1, now), Some(-1));
        assert_eq!(zoom.delta(true, true, 0., now), None);
        assert_eq!(zoom.delta(true, false, 10., now), None);
        assert_eq!(
            zoom.delta(true, true, 10., now + Duration::from_millis(499)),
            None
        );
        assert_eq!(
            zoom.delta(true, true, 10., now + Duration::from_millis(998)),
            None
        );
        assert_eq!(
            zoom.delta(true, true, 10., now + Duration::from_millis(1498)),
            Some(1)
        );
    }
}
