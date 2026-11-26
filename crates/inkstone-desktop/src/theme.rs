//! Shared visual styles for the workspace, editor, and auxiliary views.
use gpui::{Rgba, rgb, rgba};

/// Minimum size for auxiliary interface text, independent of the document font.
pub(crate) const MIN_UI_FONT_SIZE: f32 = 14.;

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: Rgba,
    pub sidebar: Rgba,
    pub surface: Rgba,
    pub foreground: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub accent: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub accent_tint: Rgba,
    pub danger_tint: Rgba,
    pub warning_tint: Rgba,
    pub highlight: Rgba,
    pub subtle_bg: Rgba,
    pub backdrop: Rgba,
}

pub(crate) fn palette(light: bool) -> Palette {
    let color = |day, night| rgb(if light { day } else { night });
    let alpha = |day, night| rgba(if light { day } else { night });
    Palette {
        background: color(0xffffff, 0x0e0e0e),
        sidebar: color(0xf5f5f5, 0x080808),
        surface: color(0xf8f8f8, 0x161616),
        foreground: color(0x0a0a0a, 0xffffff),
        muted: color(0x585858, 0xa5a5a5),
        border: color(0xe0e0e0, 0x282828),
        hover: color(0xececec, 0x1f1f1f),
        selected: color(0xe0e0e0, 0x303030),
        danger: color(0xdc2638, 0xff5c6a),
        warning: color(0xb86b00, 0xffc247),
        accent: color(0x1a73e8, 0x529eff),
        accent_tint: alpha(0x1a73e818, 0x529eff24),
        danger_tint: alpha(0xdc263818, 0xff5c6a24),
        warning_tint: alpha(0xb86b001a, 0xffc24724),
        highlight: alpha(0xffd42673, 0xffc24748),
        subtle_bg: alpha(0x0a0a0a08, 0xffffff0b),
        backdrop: alpha(0x0a0a0a5c, 0x0000009e),
    }
}
