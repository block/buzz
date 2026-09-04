/**
 * APCA 0.1.9 — the perceptual contrast algorithm in the WCAG 3 draft.
 *
 * Buzz judges contrast with this rather than the WCAG 2 ratio; see
 * DESIGN.md § Contrast for why, with the measured evidence.
 *
 * Validated against the published reference values:
 *   black on white  ->  Lc  106.04
 *   white on black  ->  Lc -107.88
 *   #888 on white   ->  Lc   63.06
 * `scripts/check-contrast.mjs` asserts these on every run, so a bad edit to the
 * maths fails loudly instead of silently shifting every verdict.
 */

const EXP = 2.4;
const R_CO = 0.2126729;
const G_CO = 0.7151522;
const B_CO = 0.072175;

const BLACK_THRESHOLD = 0.022;
// A named constant from the APCA specification that happens to read like an
// approximation of Math.SQRT2. It is not one — do not "correct" it.
// biome-ignore lint/suspicious/noApproximativeNumericConstant: APCA spec value, not Math.SQRT2
const BLACK_CLAMP = 1.414;
const SCALE = 1.14;
const DELTA_Y_MIN = 0.1;
const LO_OFFSET = 0.027;

// Dark text on a light background.
const BOW_TEXT = 0.57;
const BOW_BG = 0.56;
// Light text on a dark background — different exponents, because perception is
// not symmetric between the two polarities. This asymmetry is the whole reason
// APCA disagrees with WCAG 2 about dark mode.
const WOB_TEXT = 0.62;
const WOB_BG = 0.65;

/** Parses `#rgb` / `#rrggbb` into 0–255 channels. */
export function parseHex(hex) {
  const raw = hex.trim().replace(/^#/, "");
  const full =
    raw.length === 3
      ? raw
          .split("")
          .map((c) => c + c)
          .join("")
      : raw;
  if (!/^[0-9a-fA-F]{6}$/.test(full)) {
    throw new Error(`not an opaque hex colour: ${hex}`);
  }
  return [0, 2, 4].map((i) => Number.parseInt(full.slice(i, i + 2), 16));
}

/** Screen luminance, APCA's variant — a simple power curve, no sRGB kink. */
function screenLuminance(hex) {
  const [r, g, b] = parseHex(hex).map((c) => (c / 255) ** EXP);
  return R_CO * r + G_CO * g + B_CO * b;
}

/** Lifts near-black so tiny luminance differences don't overstate contrast. */
function clampBlack(y) {
  return y < BLACK_THRESHOLD ? y + (BLACK_THRESHOLD - y) ** BLACK_CLAMP : y;
}

/**
 * Lightness contrast between text and its background.
 *
 * Returns a signed value: positive for dark-on-light, negative for
 * light-on-dark. Compare `Math.abs()` against a target.
 */
export function apcaContrast(textHex, backgroundHex) {
  const text = clampBlack(screenLuminance(textHex));
  const background = clampBlack(screenLuminance(backgroundHex));

  if (Math.abs(background - text) < DELTA_Y_MIN) return 0;

  if (background > text) {
    const c = (background ** BOW_BG - text ** BOW_TEXT) * SCALE;
    return c < DELTA_Y_MIN ? 0 : (c - LO_OFFSET) * 100;
  }
  const c = (background ** WOB_BG - text ** WOB_TEXT) * SCALE;
  return c > -DELTA_Y_MIN ? 0 : (c + LO_OFFSET) * 100;
}

/** WCAG 2 relative luminance — reported alongside APCA, never used to decide. */
function relativeLuminance(hex) {
  const [r, g, b] = parseHex(hex).map((c) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio, for the number an audit will ask about. */
export function wcagRatio(a, b) {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}
