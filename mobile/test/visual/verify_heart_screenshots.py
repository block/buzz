#!/usr/bin/env python3
"""Check native captures of heart_sim_app.dart on iPhone 17 Pro, iOS 26.5.

Usage: python verify_heart_screenshots.py before.png after.png
Requires Pillow. Inputs are original 1206x2622 simctl screenshots, not PR crops.
"""

import sys

from PIL import Image, ImageChops


def red_pixels(image, box):
    left, top, right, bottom = box
    return [
        (x, y)
        for y in range(top, bottom)
        for x in range(left, right)
        if is_red(image.getpixel((x, y)))
    ]


def is_red(color):
    red, green, blue = color
    return red > 140 and red > green * 1.5 and red > blue * 1.5


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
        "italic message heart": (355, 1040, 425, 1090),
        "emoji-only message": (65, 1490, 125, 1545),
    }
    for name, box in regions.items():
        assert not red_pixels(before, box), f"{name}: original must reproduce monochrome heart"
        pixels = red_pixels(after, box)
        assert len(pixels) > 500, f"{name}: expected native red heart"
        if "reaction" in name:
            center = (min(y for _, y in pixels) + max(y for _, y in pixels)) / 2
            pill_center = (box[1] + box[3] - 1) / 2
            assert abs(center - pill_center) <= 1.5, f"{name}: heart is not centered"
        print(f"PASS: {name}")
    # The two preceding message lines gain one logical pixel each from the
    # fallback font's metrics. Compare unchanged text after that translation.
    for name, box in {
        "warning signs": (65, 1090, 1000, 1145),
        "preserved symbols and languages": (65, 1260, 1100, 1385),
    }.items():
        shifted = (box[0], box[1] + 6, box[2], box[3] + 6)
        difference = ImageChops.difference(before.crop(box), after.crop(shifted))
        assert difference.getbbox() is None, f"{name}: unexpected rendering change"
        print(f"PASS: {name} is pixel-identical")


if __name__ == "__main__":
    main()
