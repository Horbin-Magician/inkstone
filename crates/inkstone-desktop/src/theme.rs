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
        background: color(0xffffff, 0x0d0e12),
        sidebar: color(0xf4f5f8, 0x07080a),
        surface: color(0xf7f8fa, 0x14161c),
        foreground: color(0x090a0d, 0xffffff),
        muted: color(0x525866, 0x9da5b4),
        border: color(0xdce0e8, 0x242833),
        hover: color(0xe9ecf2, 0x1c1f28),
        selected: color(0xe5f0ff, 0x142847),
        danger: color(0xdc2638, 0xff5c6a),
        warning: color(0xb86b00, 0xffc247),
        accent: color(0x1a73e8, 0x529eff),
        accent_tint: alpha(0x1a73e818, 0x529eff24),
        danger_tint: alpha(0xdc263818, 0xff5c6a24),
        warning_tint: alpha(0xb86b001a, 0xffc24724),
        highlight: alpha(0xffd42673, 0xffc24748),
        subtle_bg: alpha(0x090a0d08, 0xffffff0b),
        backdrop: alpha(0x090a0d5c, 0x0000009e),
    }
}
