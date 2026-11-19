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
    pub subtle_bg: Rgba,
    pub backdrop: Rgba,
}

pub(crate) fn palette(light: bool) -> Palette {
    let color = |day, night| rgb(if light { day } else { night });
    let alpha = |day, night| rgba(if light { day } else { night });
    Palette {
        background: color(0xffffff, 0x1a1d21),
        sidebar: color(0xf5f7fa, 0x14171b),
        surface: color(0xf8fafc, 0x22262d),
        foreground: color(0x1e293b, 0xe8edf3),
        muted: color(0x64748b, 0x8c98a9),
        border: color(0xe3e8ef, 0x2d333d),
        hover: color(0xedf2f7, 0x282d36),
        selected: color(0xe2f0f4, 0x213942),
        danger: color(0xb93846, 0xf8969b),
        warning: color(0x9a621a, 0xf0c178),
        accent: color(0x1f7588, 0x6bc2d4),
        accent_tint: alpha(0x1f758816, 0x6bc2d420),
        danger_tint: alpha(0xb9384616, 0xf8969b20),
        warning_tint: alpha(0x9a621a18, 0xf0c17820),
        subtle_bg: alpha(0x0f172a06, 0xffffff06),
        backdrop: alpha(0x0f172a52, 0x00000078),
    }
}
