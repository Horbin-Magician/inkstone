"""Render the DMG instruction artwork on macOS with Pillow 11.3.0."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont


def main():
    # A two-resolution TIFF keeps Finder text sharp on Retina displays.
    frames = []
    for scale in (1, 2):
        image = Image.new("RGB", (640 * scale, 400 * scale), "#F5F4F0")
        draw = ImageDraw.Draw(image)
        font_path = "/System/Library/Fonts/STHeiti Light.ttc"
        def text(y, label, size, color):
            font = ImageFont.truetype(font_path, size * scale)
            draw.text((320 * scale, y * scale), label, font=font, fill=color, anchor="mt")
        text(45, "安装墨砚", 25, "#292B2D")
        text(290, "将墨砚拖入 Applications 文件夹", 17, "#454749")
        text(324, "安装后，从“应用程序”打开墨砚", 13, "#7B7D7E")
        points = [(294, 172), (346, 172)]
        draw.line([(x * scale, y * scale) for x, y in points], fill="#A1A39F", width=2 * scale)
        draw.line([(337 * scale, 163 * scale), (346 * scale, 172 * scale),
                   (337 * scale, 181 * scale)], fill="#A1A39F", width=2 * scale)
        frames.append(image)
    frames[0].save(Path(__file__).with_name("background.tiff"),
                   save_all=True, append_images=frames[1:], compression="tiff_lzw")


if __name__ == "__main__":
    main()
