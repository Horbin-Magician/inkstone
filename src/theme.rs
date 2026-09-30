//! Shared semantic colors for the workspace, editor, and auxiliary views.
use gpui::{Rgba, rgb};

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
        background: color(0xffffff, 0x1c2027),
        sidebar: color(0xf4f6f8, 0x171b21),
        surface: color(0xf8fafb, 0x242a33),
        foreground: color(0x263342, 0xe2e8ef),
        muted: color(0x627184, 0x9aa8b9),
        border: color(0xe2e8ef, 0x303946),
        hover: color(0xeaf0f5, 0x2b3441),
        selected: color(0xe0eef2, 0x233e48),
        accent: color(0x24778a, 0x72bfd0),
    }
}
