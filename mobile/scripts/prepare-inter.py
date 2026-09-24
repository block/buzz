#!/usr/bin/env python3
"""Remove only Inter's U+2764 mapping so hearts use native font fallback.

Requires fonttools==4.60.1. See assets/fonts/README.md for source and commands.
"""

import argparse
import hashlib
from pathlib import Path

from fontTools.ttLib import TTFont


SOURCES = {
    "InterVariable.ttf": "4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf",
    "InterVariable-Italic.ttf": "d6f1f6a172d9e588438db9f986fd5cfad7b30f644374080a8a9d4d91e344586f",
}
HEART = 0x2764


def verify(source: Path, output: Path) -> None:
    with TTFont(source, lazy=True) as before, TTFont(output, lazy=True) as after:
        if set(before.reader.keys()) != set(after.reader.keys()):
            raise ValueError("Font tables changed")
        for tag in before.reader.keys():
            original, modified = before.reader[tag], after.reader[tag]
            if tag == "cmap":
                continue
            if tag == "head":
                # The only allowed header change is the whole-font checksum.
                original = original[:8] + original[12:]
                modified = modified[:8] + modified[12:]
            if original != modified:
                raise ValueError(f"Unexpected change to {tag}")
        old_tables, new_tables = before["cmap"].tables, after["cmap"].tables
        if len(old_tables) != len(new_tables):
            raise ValueError("Character-map table count changed")
        for old, new in zip(old_tables, new_tables, strict=True):
            identity = lambda t: (t.format, t.platformID, t.platEncID, t.language)
            if identity(old) != identity(new):
                raise ValueError("Character-map identity changed")
            expected = dict(old.cmap)
            if old.isUnicode():
                del expected[HEART]
            if new.cmap != expected:
                raise ValueError("Character mappings changed beyond U+2764")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_dir", type=Path)
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).resolve().parents[1] / "assets/fonts")
    parser.add_argument("--check", action="store_true", help="Verify existing output without writing")
    args = parser.parse_args()
    for name, digest in SOURCES.items():
        source, output = args.source_dir / name, args.output_dir / name
        if source.resolve() == output.resolve():
            raise ValueError("Keep pristine sources separate from generated fonts")
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise ValueError(f"Unexpected source font: {name}")
        if not args.check:
            with TTFont(source, lazy=True, recalcTimestamp=False) as font:
                tables = [t for t in font["cmap"].tables if t.isUnicode()]
                if not tables or any(HEART not in t.cmap for t in tables):
                    raise ValueError(f"Expected heart mapping in {name}")
                # FontTools shares dictionaries between equivalent subtables.
                for cmap in {id(t.cmap): t.cmap for t in tables}.values():
                    del cmap[HEART]
                args.output_dir.mkdir(parents=True, exist_ok=True)
                font.save(output, reorderTables=False)
        verify(source, output)
        print(f"{name}: only U+2764 mapping removed; other mappings and glyph data unchanged")


if __name__ == "__main__":
    main()
