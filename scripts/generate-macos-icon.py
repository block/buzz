#!/usr/bin/env python3
"""Build the legacy macOS icon without relying on system corner masking.

Run: scripts/macos-icon.sh [--check]

The wrapper pins uv, Python and Pillow. Verification compares the complete set
of decoded RGBA representations, not PNG encoding or ICNS container bytes:
Apple's system iconutil may encode the same pixels differently across macOS.
The shared artwork and other platforms' icons are intentionally untouched.
"""

# /// script
# requires-python = "==3.14.3"
# dependencies = ["Pillow==12.2.0"]
# ///

import argparse
from pathlib import Path
import subprocess
import tempfile

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "desktop/src-tauri/icons"


def generate(output):
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
            ["/usr/bin/iconutil", "-c", "icns", str(iconset), "-o", str(output)],
            check=True,
        )


def verify(candidate, reference):
    """Reject missing/extra representations and any decoded pixel drift."""
    with tempfile.TemporaryDirectory(prefix="buzz-icon-check-") as temporary:
        sets = []
        for name, icon in (("candidate", candidate), ("reference", reference)):
            iconset = Path(temporary) / f"{name}.iconset"
            subprocess.run(
                ["/usr/bin/iconutil", "-c", "iconset", str(icon), "-o", str(iconset)],
                check=True,
            )
            sets.append({p.name: p for p in iconset.glob("*.png")})
        if sets[0].keys() != sets[1].keys():
            raise ValueError("macOS icon representation set drifted")
        for name in sorted(sets[0]):
            with Image.open(sets[0][name]) as a, Image.open(sets[1][name]) as b:
                if a.size != b.size or a.convert("RGBA").tobytes() != b.convert("RGBA").tobytes():
                    raise ValueError(f"macOS icon pixel drift: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without modifying the icon")
    parser.add_argument("--output", type=Path, default=ICONS / "icon.icns")
    args = parser.parse_args()
    if args.check:
        with tempfile.TemporaryDirectory(prefix="buzz-icon-regenerate-") as temporary:
            generated = Path(temporary) / "icon.icns"
            generate(generated)
            verify(args.output, generated)
        print("macOS icon: all decoded representations match")
    else:
        generate(args.output)


if __name__ == "__main__":
    main()
