//! Shared visual styles for the workspace, editor, and auxiliary views.
use gpui::{Rgba, rgb};

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
}

pub(crate) fn palette(light: bool) -> Palette {
    let color = |day, night| rgb(if light { day } else { night });
    Palette {
        background: color(0xffffff, 0x212121),
        sidebar: color(0xf4f6f8, 0x1b1b1b),
        surface: color(0xf8fafb, 0x292929),
        foreground: color(0x263342, 0xe8e8e8),
        muted: color(0x627184, 0xadadad),
        border: color(0xe2e8ef, 0x383838),
        hover: color(0xeaf0f5, 0x303030),
        selected: color(0xe0eef2, 0x3a3a3a),
        accent: color(0x24778a, 0x72bfd0),
    }
}
