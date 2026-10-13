"""Generate Zelamar app icons from the Sunset Z emblem geometry.

The geometry mirrors `app/lib/src/design/emblem.dart` (200x200 design grid),
so the launcher icon and the in-app emblem are the same drawing. Run after
any change to the emblem:

    python scripts/generate_icons.py

Requires Pillow. Writes:
- Android legacy launcher icons (mipmap-*/ic_launcher.png)
- Android adaptive icon foreground + monochrome layers (API 26+)
- Windows app_icon.ico
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ANDROID_RES = ROOT / "app" / "android" / "app" / "src" / "main" / "res"
WINDOWS_ICO = ROOT / "app" / "windows" / "runner" / "resources" / "app_icon.ico"

TILE = (0xF8, 0xF3, 0xEA, 255)  # emblemTile, light theme
MARK = (0x8A, 0x5A, 0x30, 255)  # emblemMark, light theme
GRID = 200
TILE_RADIUS = 46
SUPERSAMPLE = 8

FULL_LINES = [((48, 94), (152, 94)), ((106, 108), (140, 108)), ((83, 122), (117, 122)),
              ((60, 136), (94, 136)), ((48, 150), (152, 150))]
COMPACT_LINES = [((46, 100), (154, 100)), ((78, 130), (122, 130)), ((46, 160), (154, 160))]


def stroke_width(size: int) -> int:
    """Stroke width on the design grid; thicker as the icon gets smaller."""
    if size < 40:
        return 14
    if size < 80:
        return 9
    return 5


def draw_mark(draw: ImageDraw.ImageDraw, size: int, scale: float, offset: float) -> None:
    """Draw the Sunset Z mark in grid units mapped by `scale` and `offset`."""
    compact = size < 40

    def p(x: float, y: float) -> tuple[float, float]:
        return (offset + x * scale, offset + y * scale)

    sun_r = 36 if compact else 30
    cx, cy = 100, 84
    draw.pieslice([p(cx - sun_r, cy - sun_r), p(cx + sun_r, cy + sun_r)], 180, 360, fill=MARK)

    width = stroke_width(size) * scale
    for (x1, y1), (x2, y2) in COMPACT_LINES if compact else FULL_LINES:
        a, b = p(x1, y1), p(x2, y2)
        draw.line([a, b], fill=MARK, width=round(width))
        r = width / 2
        for x, y in (a, b):  # round caps
            draw.ellipse([x - r, y - r, x + r, y + r], fill=MARK)


def render_tile(size: int) -> Image.Image:
    """Full emblem on its rounded tile, transparent corners."""
    big = size * SUPERSAMPLE
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    scale = big / GRID
    draw.rounded_rectangle([0, 0, big - 1, big - 1], radius=TILE_RADIUS * scale, fill=TILE)
    draw_mark(draw, size, scale, 0)
    return img.resize((size, size), Image.LANCZOS)


def render_adaptive_foreground(px: int) -> Image.Image:
    """Mark only, on a 108dp canvas: the 200-unit grid maps onto the central
    84dp, which keeps the farthest point of the mark ~30dp from the centre,
    inside Android's 33dp safe-zone radius."""
    big = px * SUPERSAMPLE
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    scale = big * (84 / 108) / GRID
    offset = big * (12 / 108)
    draw_mark(draw, 200, scale, offset)
    return img.resize((px, px), Image.LANCZOS)


def main() -> None:
    densities = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}
    for name, factor in densities.items():
        folder = ANDROID_RES / f"mipmap-{name}"
        folder.mkdir(parents=True, exist_ok=True)
        render_tile(round(48 * factor)).save(folder / "ic_launcher.png", optimize=True)
        fg = render_adaptive_foreground(round(108 * factor))
        fg.save(folder / "ic_launcher_foreground.png", optimize=True)
        fg.save(folder / "ic_launcher_monochrome.png", optimize=True)

    sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    icons = [render_tile(s) for s in sizes]
    icons[-1].save(WINDOWS_ICO, format="ICO", sizes=[(s, s) for s in sizes],
                   append_images=icons[:-1])
    print(f"Android icons -> {ANDROID_RES}\nWindows icon  -> {WINDOWS_ICO}")


if __name__ == "__main__":
    main()
