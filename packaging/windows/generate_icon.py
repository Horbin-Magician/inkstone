"""Generate the Windows ICO from the shared artwork (requires Pillow).

The source PNG includes macOS Dock padding. Windows already reserves space
around taskbar icons, so retaining that padding makes the app look too small.
"""

from pathlib import Path

from PIL import Image


ASSETS = Path(__file__).resolve().parents[2] / "crates/inkstone-desktop/assets"
SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)


def main():
    with Image.open(ASSETS / "inkstone-icon.png") as source:
        artwork = source.convert("RGBA")
    # Ignore almost-transparent shadow/noise outside the actual tile. Using
    # the raw alpha bounds would retain most of the unwanted source padding.
    bounds = artwork.getchannel("A").point(lambda a: 255 if a >= 128 else 0).getbbox()
    if bounds is None:
        raise ValueError("Application icon has no visible artwork")
    left, top, right, bottom = bounds
    edge = max(right - left, bottom - top)
    left -= (edge - (right - left)) // 2
    top -= (edge - (bottom - top)) // 2
    # A square crop preserves the original proportions and rounded corners.
    artwork = artwork.crop((left, top, left + edge, top + edge))
    artwork.save(
        ASSETS / "inkstone.ico",
        format="ICO",
        sizes=[(size, size) for size in SIZES],
    )


if __name__ == "__main__":
    main()
