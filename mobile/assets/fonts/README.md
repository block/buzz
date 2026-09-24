# Inter heart and warning fallback

The two Inter 4.001 variable fonts are derived from the previously bundled
fonts by removing only U+2764 and U+26A0 from their Unicode character maps.
Flutter can then choose a native font for heart and warning characters,
including emoji presentation
in reactions and message text. No glyphs, language coverage, other symbols,
font metrics, or variation tables are removed. The license remains in
`Inter-LICENSE.txt`.

To reproduce from the repository root:

```sh
mkdir -p /tmp/buzz-inter-source
for font in InterVariable.ttf InterVariable-Italic.ttf; do
  git show ec7ea38f62ea917f15e85a678bc94f3bbee5bb64:mobile/assets/fonts/$font > /tmp/buzz-inter-source/$font
done
python3 -m venv /tmp/buzz-inter-tools
/tmp/buzz-inter-tools/bin/pip install fonttools==4.60.1
/tmp/buzz-inter-tools/bin/python mobile/scripts/prepare-inter.py /tmp/buzz-inter-source
/tmp/buzz-inter-tools/bin/python mobile/scripts/prepare-inter.py /tmp/buzz-inter-source --check
```

The script checks pristine source hashes and verifies every character mapping
and every other font table. Only the `cmap` table and the font checksum may
change. Repeated generation from the same sources produces identical bytes.
©, ®, ™, arrows, math symbols, accents, and supported languages
keep their original Inter mappings.

Use `test/visual/heart_sim_app.dart` on an actual iOS simulator to compare
reactions, plain/emoji/text-presentation hearts, bold/italic message text,
warning signs, and preserved symbols. Font fallback for plain and explicit
text-presentation hearts and warnings is platform-dependent; do not infer their appearance
from a macOS widget-test renderer. On iOS 26.5, all three heart and warning
presentations fall back to the color emoji, including explicit VS15. Message line metrics
can change with the fallback font.

The native screenshot regression check (requires Pillow) covers both reaction
states, each heart and warning presentation, bold/italic text, and unchanged
symbols:

```sh
python3 mobile/test/visual/verify_heart_screenshots.py before.png after.png
```

Use original 1206×2622 captures from the documented simulator and fixture for
this check. PR images may be cropped afterward for readability.
