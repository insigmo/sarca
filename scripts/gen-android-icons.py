#!/usr/bin/env python3
"""Regenerate Android launcher icons with a safe-zone-padded goose.

`tauri icon` builds every platform's icons by resizing `logo.svg` edge to
edge, which is correct for desktop/iOS/Windows icons but wrong for Android:
the adaptive-icon mask only shows the central 72/108 (~66%) of the
foreground layer, so a full-bleed goose gets its head and feet cropped off
(only the neck and breast survive). This script re-renders the goose scaled
down and centered so the whole bird sits inside that mask, then overwrites
the `ic_launcher*` PNGs `tauri icon` just wrote, at the exact sizes it wrote
them (nothing here hardcodes a resolution table, so it keeps working if
that changes).

`logo.svg` draws the goose from two stacked raster layers rather than one
flat image -- a filled base (`img1`: white body, orange beak/feet) and a
black outline overlay (`img2`), each placed via its own `<use x= y=>`
offset. Both must be composited (in that order) to get "the goose" the rest
of the app renders; using either PNG alone gives an outline-only or
edge-bleeding result.

Run via `task ico:update` (after `pnpm exec tauri icon`), or directly:
    python3 scripts/gen-android-icons.py
"""
from __future__ import annotations

import base64
import io
import re
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
LOGO_SVG = ROOT / "logo.svg"
ANDROID_ICONS = ROOT / "client" / "src-tauri" / "icons" / "android"
BACKGROUND_XML = ANDROID_ICONS / "values" / "ic_launcher_background.xml"

# Fraction of the canvas the goose's longest side occupies once centered.
# Android's adaptive-icon mask crops the foreground layer to its central
# 72/108 (~66.7%), so the foreground goose needs real margin inside that —
# 58% leaves roughly 4 points of slack on each side. The legacy
# (non-adaptive) icon has no mask, so it can sit closer to the edge.
FOREGROUND_SCALE = 0.58
LEGACY_SCALE = 0.76


def hex_color_to_rgba(value: str) -> tuple[int, int, int, int]:
    """Parse `#fff` / `#ffffff` / `#ffffffff` into an RGBA tuple."""
    v = value.strip().lstrip("#")
    if len(v) == 3:
        v = "".join(c * 2 for c in v)
    if len(v) == 6:
        v += "ff"
    if len(v) != 8:
        raise SystemExit(f"unsupported color format: {value!r}")
    r, g, b, a = (int(v[i : i + 2], 16) for i in range(0, 8, 2))
    return (r, g, b, a)


def background_color() -> tuple[int, int, int, int]:
    text = BACKGROUND_XML.read_text(encoding="utf-8")
    m = re.search(r'name="ic_launcher_background">([^<]+)<', text)
    if not m:
        raise SystemExit(f"could not find ic_launcher_background color in {BACKGROUND_XML}")
    return hex_color_to_rgba(m.group(1))


def extract_layer(svg_text: str, image_id: str) -> tuple[Image.Image, int, int]:
    """Decode one embedded `<image id=...>` and its `<use href="#...">`
    placement offset."""
    img_m = re.search(
        rf'<image\s+width="(\d+)"\s+height="(\d+)"\s+id="{image_id}"\s+'
        rf'href="data:image/png;base64,([^"]+)"',
        svg_text,
    )
    if not img_m:
        raise SystemExit(f'could not find <image id="{image_id}"> in {LOGO_SVG}')
    _w, _h, b64 = img_m.groups()
    image = Image.open(io.BytesIO(base64.b64decode(b64))).convert("RGBA")

    use_m = re.search(
        rf'<use\s+id="[^"]*"\s+href="#{image_id}"\s+x="(-?\d+)"\s+y="(-?\d+)"', svg_text
    )
    if not use_m:
        raise SystemExit(f'could not find <use href="#{image_id}"> in {LOGO_SVG}')
    x, y = (int(v) for v in use_m.groups())
    return image, x, y


def render_goose(svg_text: str) -> Image.Image:
    """Composite `img1` (fill) under `img2` (outline) onto the SVG's own
    viewBox canvas -- the exact picture `logo.svg` renders -- then return it
    tightly cropped to its opaque bounding box."""
    vb_m = re.search(r'viewBox="0 0 (\d+) (\d+)"', svg_text)
    if not vb_m:
        raise SystemExit(f"could not find viewBox in {LOGO_SVG}")
    vb_w, vb_h = (int(v) for v in vb_m.groups())

    composited = Image.new("RGBA", (vb_w, vb_h), (0, 0, 0, 0))
    for image_id in ("img1", "img2"):
        layer, x, y = extract_layer(svg_text, image_id)
        layer_canvas = Image.new("RGBA", (vb_w, vb_h), (0, 0, 0, 0))
        # `paste` clips silently when (x, y) + layer size overflows the
        # canvas, same as an SVG viewBox would.
        layer_canvas.paste(layer, (x, y), layer)
        composited = Image.alpha_composite(composited, layer_canvas)

    bbox = composited.getbbox()
    if bbox is None:
        raise SystemExit("composited goose is fully transparent -- did logo.svg change shape?")
    return composited.crop(bbox)


def centered_on(
    goose: Image.Image,
    canvas_size: int,
    scale: float,
    background: tuple[int, int, int, int],
) -> Image.Image:
    """Center `goose` on a `canvas_size`^2 square so its longest side is
    `scale` of the canvas."""
    canvas = Image.new("RGBA", (canvas_size, canvas_size), background)
    gw, gh = goose.size
    target = max(1, round(canvas_size * scale))
    ratio = target / max(gw, gh)
    new_size = (max(1, round(gw * ratio)), max(1, round(gh * ratio)))
    resized = goose.resize(new_size, Image.LANCZOS)
    offset = ((canvas_size - new_size[0]) // 2, (canvas_size - new_size[1]) // 2)
    canvas.paste(resized, offset, resized)
    return canvas


def circle_masked(square: Image.Image) -> Image.Image:
    """Clip an opaque square to the circle inscribed in it (Android's
    `ic_launcher_round` convention)."""
    size = square.size
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).ellipse((0, 0, size[0] - 1, size[1] - 1), fill=255)
    out = square.copy()
    out.putalpha(mask)
    return out


def main() -> None:
    svg_text = LOGO_SVG.read_text(encoding="utf-8")
    goose = render_goose(svg_text)
    background = background_color()
    transparent = (0, 0, 0, 0)

    density_dirs = sorted(ANDROID_ICONS.glob("mipmap-*dpi"))
    if not density_dirs:
        raise SystemExit(f"no mipmap-*dpi directories under {ANDROID_ICONS}")

    for density_dir in density_dirs:
        fg_path = density_dir / "ic_launcher_foreground.png"
        if fg_path.exists():
            size = Image.open(fg_path).size[0]
            centered_on(goose, size, FOREGROUND_SCALE, transparent).save(fg_path)
            print(f"wrote {fg_path.relative_to(ROOT)} ({size}x{size}, goose @ {FOREGROUND_SCALE:.0%})")

        legacy_path = density_dir / "ic_launcher.png"
        if legacy_path.exists():
            size = Image.open(legacy_path).size[0]
            centered_on(goose, size, LEGACY_SCALE, background).save(legacy_path)
            print(f"wrote {legacy_path.relative_to(ROOT)} ({size}x{size}, goose @ {LEGACY_SCALE:.0%})")

        round_path = density_dir / "ic_launcher_round.png"
        if round_path.exists():
            size = Image.open(round_path).size[0]
            square = centered_on(goose, size, LEGACY_SCALE, background)
            circle_masked(square).save(round_path)
            print(f"wrote {round_path.relative_to(ROOT)} ({size}x{size}, circle-masked)")


if __name__ == "__main__":
    main()
