#!/usr/bin/env python3
"""Build the legacy macOS icon without relying on system corner masking.

Run on macOS with Pillow installed: python3 scripts/generate-macos-icon.py
The shared artwork and other platforms' icons are intentionally untouched.
"""

from pathlib import Path
import subprocess
import tempfile

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "desktop/src-tauri/icons"


def main():
    # An 824px tile on a 1024px canvas leaves the macOS optical inset.
    # Work at 4x resolution to antialias the rounded edge at every icon size.
    scale = 4
    canvas = Image.new("RGBA", (1024 * scale, 1024 * scale))
    with Image.open(ICONS / "buzz-source.png") as source:
        tile = source.convert("RGBA").resize(
            (824 * scale, 824 * scale), Image.Resampling.LANCZOS
        )
    mask = Image.new("L", tile.size)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, tile.width - 1, tile.height - 1), radius=185 * scale, fill=255
    )
    canvas.paste(tile, (100 * scale, 100 * scale), mask)

    with tempfile.TemporaryDirectory(prefix="buzz-macos-icon-") as temporary:
        iconset = Path(temporary) / "Buzz.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for density in (1, 2):
                suffix = "@2x" if density == 2 else ""
                canvas.resize(
                    (size * density, size * density), Image.Resampling.LANCZOS
                ).save(iconset / f"icon_{size}x{size}{suffix}.png")
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")],
            check=True,
        )


if __name__ == "__main__":
    main()
