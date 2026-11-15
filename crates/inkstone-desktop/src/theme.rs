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
    pub danger: Rgba,
    pub warning: Rgba,
}

pub(crate) fn palette(light: bool) -> Palette {
    let color = |day, night| rgb(if light { day } else { night });
    Palette {
        background: color(0xfcfdfc, 0x202826),
        sidebar: color(0xeff3f1, 0x19211f),
        surface: color(0xf5f7f5, 0x27312e),
        foreground: color(0x233330, 0xe2ebe5),
        muted: color(0x586c64, 0xa6b7ad),
        border: color(0xd7e0da, 0x3e4d46),
        hover: color(0xe5ece7, 0x303e38),
        selected: color(0xd3e7dc, 0x324d42),
        danger: color(0xa43540, 0xffa8ab),
        warning: color(0x885518, 0xf0c184),
        accent: color(0x23695f, 0x8ad2b8),
    }
}
