"""Regenerate NSIS artwork with Pillow; normal packaging uses committed BMPs."""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
ASSETS = Path(__file__).resolve().parent / "assets"
PAPER = "#F5F4F0"
INK = "#292B2D"


def artwork(width, height, icon_size, icon_position):
    # Render at 2x for crisp edges when Windows scales the installer for HiDPI.
    canvas = Image.new("RGB", (width * 2, height * 2), PAPER)
    icon = Image.open(ROOT / "crates/inkstone-desktop/assets/inkstone-icon.png").convert("RGBA")
    icon.thumbnail((icon_size * 2, icon_size * 2), Image.Resampling.LANCZOS)
    canvas.paste(icon, tuple(v * 2 for v in icon_position), icon)
    return canvas


def main():
    ASSETS.mkdir(exist_ok=True)
    sidebar = artwork(164, 314, 112, (26, 65))
    draw = ImageDraw.Draw(sidebar)
    # Quiet rules echo the lines of a notebook without embedding localized text.
    for y, length in ((211, 56), (223, 40), (235, 28)):
        draw.rounded_rectangle((108, y * 2, (54 + length) * 2, y * 2 + 3),
                               radius=1, fill="#C5C4BF")
    sidebar.save(ASSETS / "sidebar.bmp")
    header = Image.new("RGB", (300, 114), "white")
    icon = Image.open(ROOT / "crates/inkstone-desktop/assets/inkstone-icon.png").convert("RGBA")
    icon.thumbnail((96, 96), Image.Resampling.LANCZOS)
    header.paste(icon, (184, 9), icon)
    header.save(ASSETS / "header.bmp")


if __name__ == "__main__":
    main()
