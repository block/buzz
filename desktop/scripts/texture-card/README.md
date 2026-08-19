# Textured Card asset

`Card variant="textured"` renders a cheap CSS nine-slice PNG at runtime. This
folder preserves the procedural SVG recipe used to generate that asset.

## Regenerate

From `desktop/`:

```bash
pnpm exec node scripts/texture-card/generate-card-texture.mjs
```

This overwrites:

```text
src/shared/ui/assets/card-texture.png
```

The generator renders the archived SVG filter in headless Chromium at 2× DPR,
then captures a transparent PNG. It is a development tool only and is not
included in the production bundle.

After changing texture parameters:

1. Regenerate the asset.
2. Compare the onboarding private-key card at its normal size.
3. Check a tall/narrow textured Card and the smallest supported shape.
4. Check both Retina and standard-density displays.
5. Update the slice/outset values in `card-texture.css` only if the generated
   geometry changed.

The runtime component API remains `Card variant="textured"`; feature code owns
padding, dimensions, typography, and placement.
