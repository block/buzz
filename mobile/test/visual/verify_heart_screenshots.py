#!/usr/bin/env python3
"""Check native captures of heart_sim_app.dart on iPhone 17 Pro, iOS 26.5.

Usage: python verify_heart_screenshots.py before.png after.png
Requires Pillow. Inputs are original 1206x2622 simctl screenshots, not PR crops.
"""

import sys

from PIL import Image, ImageChops


def color_pixels(image, box, predicate):
    left, top, right, bottom = box
    return [
        (x, y)
        for y in range(top, bottom)
        for x in range(left, right)
        if predicate(image.getpixel((x, y)))
    ]


def is_red(color):
    red, green, blue = color
    return red > 140 and red > green * 1.5 and red > blue * 1.5


def is_yellow(color):
    red, green, blue = color
    return red > 180 and green > 130 and blue < 90


def main():
    before, after = [Image.open(path).convert("RGB") for path in sys.argv[1:]]
    assert before.size == after.size == (1206, 2622), "Use the documented simulator and fixture"
    regions = {
        "selected reaction": (95, 522, 150, 606),
        "unselected reaction": (95, 744, 150, 828),
        "plain message heart": (175, 980, 225, 1035),
        "emoji-presentation message heart": (375, 980, 435, 1035),
        # Native iOS fallback currently renders VS15 as a color heart too.
        "text-presentation message heart": (550, 980, 620, 1035),
        "bold message heart": (170, 1040, 220, 1090),
        "italic message heart": (430, 1040, 490, 1090),
        "emoji-only message": (65, 1490, 200, 1650),
    }
    warnings = {
        "selected warning reaction": (260, 522, 325, 606),
        "unselected warning reaction": (260, 744, 325, 828),
        "plain message warning": (275, 1090, 335, 1150),
        "emoji-presentation message warning": (350, 1090, 415, 1150),
        "text-presentation message warning": (430, 1090, 490, 1150),
        "bold message warning": (230, 1040, 290, 1090),
        "italic message warning": (490, 1040, 550, 1090),
        "emoji-only warning": (205, 1490, 335, 1650),
    }
    for cases, predicate in ((regions, is_red), (warnings, is_yellow)):
        for name, box in cases.items():
            assert not color_pixels(before, box, predicate), f"{name}: original must reproduce monochrome glyph"
            pixels = color_pixels(after, box, predicate)
            assert len(pixels) > 500, f"{name}: expected native color emoji"
            if "emoji-only" in name:
                # Production uses a separate 36sp path. Reject a fixture that
                # accidentally renders normal-sized inline emoji or clips it.
                left, right = min(x for x, _ in pixels), max(x for x, _ in pixels)
                top, bottom = min(y for _, y in pixels), max(y for _, y in pixels)
                assert len(pixels) > 4500, f"{name}: expected enlarged emoji"
                assert 90 <= right - left <= 110 and 90 <= bottom - top <= 110
                assert box[0] < left < right < box[2] - 1
                assert box[1] < top < bottom < box[3] - 1
            if "reaction" in name:
                center = (min(y for _, y in pixels) + max(y for _, y in pixels)) / 2
                pill_center = (box[1] + box[3] - 1) / 2
                assert abs(center - pill_center) <= 1.5, f"{name}: emoji is not centered"
            print(f"PASS: {name}")
    # The three preceding message lines gain one logical pixel each from the
    # fallback font's metrics. Compare unchanged text after that translation.
    for name, box in {
        "preserved symbols and languages": (65, 1260, 1100, 1385),
    }.items():
        shifted = (box[0], box[1] + 9, box[2], box[3] + 9)
        difference = ImageChops.difference(before.crop(box), after.crop(shifted))
        assert difference.getbbox() is None, f"{name}: unexpected rendering change"
        print(f"PASS: {name} is pixel-identical")


if __name__ == "__main__":
    main()
