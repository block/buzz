/**
 * Terminal banner animation — the MOTION half of the banner contract.
 *
 * Geometry (terminalBanner.ts) is static and colour-free; colour
 * (terminalBannerColor.ts) maps (layer, t, hue) -> hex. This module supplies
 * the `hue` axis as a function of time and screen position, plus the lookup
 * table that makes it affordable.
 *
 * Tyler, amendment (msg 25bf80a2) — AUTHORITATIVE: the field wave
 * "originates bottom right and travels top left", not plain right-to-left.
 * This SUPERSEDES the original directive: "slow waves of the buzz theme colors
 * moving across the honeycomb lattice right to left, with a separate slow
 * color wave on the `buzz term` going left to right". The wordmark half is
 * unchanged by the amendment; only the field's axis moved.
 *
 * Two independent waves, different axes, different periods:
 *   - field    -> originates bottom-right, travels top-left across the whole
 *                 viewport (see FIELD_WAVE.direction = {x:-1, y:-1})
 *   - wordmark -> left-to-right across the wordmark's own ink span
 *
 * LOOP SEMANTICS. Phase advances monotonically and wraps at 1 -> 0 (`phaseAt`
 * via `wrap01`); travel is continuous and one-way for as long as it advances,
 * and one full cycle translates the pattern by exactly one spatial period, so
 * the wrap is C0 with no snap and no reversal. The palette axis is folded
 * (`cyclicRampPosition`) so the open three-stop ramp can be driven by a cyclic
 * phase without a seam. Proof that folding the palette does NOT fold the motion
 * is on `cyclicRampPosition` -- read it before touching either.
 *
 * The bevels do not animate. They are the chassis; a moving highlight would
 * read as the box itself deforming.
 */
import type { TerminalPalette } from "../../shared/theme/terminal-palette";
import {
  type Layer,
  bannerColor,
  bannerStops,
  hex2rgb,
  lift,
  rgb2hex,
} from "./terminalBannerColor";

/** WCAG 2.2 SC 1.4.3 body text, the wordmark's floor. Re-declared here because
 *  the interpolated blend has to be re-lifted to it; see `sampleHead`. */
const WORDMARK_FLOOR = 4.5;

/**
 * ============================================================================
 * TUNING. Everything Tyler can retune lives in this one block.
 * ============================================================================
 *
 * Two independent waves, one config each. Every knob states its unit and what
 * "bigger" does. Nothing else in this file needs editing to retune the
 * animation.
 *
 * `direction` is a vector in VISUAL space, not cell space. This matters: a
 * terminal cell is 8.4 x 17 CSS px, so it is about 2:1 tall, and a naive
 * (-1,-1) over cell indices would render as a shallow 27-degree wave rather
 * than the intended diagonal. `projectWave` converts to visual space before
 * projecting, so a 45-degree vector here looks like 45 degrees on screen.
 */
export type BannerWaveConfig = {
  /**
   * Travel direction in visual space; x grows right, y grows DOWN. The wave
   * moves TOWARD this vector, so it appears to originate from the opposite
   * corner. Magnitude is irrelevant — it is normalised.
   */
  direction: { x: number; y: number };
  /**
   * Seconds for one full traversal of the colour ramp. BIGGER = SLOWER.
   * This is ambience, not a screensaver: the direction gate is indifferent to
   * this value but the "slow" gate requires it to stay above 5s.
   */
  seconds: number;
  /**
   * How much of the colour ramp is visible across one screen-width of travel,
   * in ramp traversals. BIGGER = MORE COLOUR BANDS ON SCREEN AT ONCE (a
   * tighter, busier wave); smaller = a broader, calmer wash.
   *
   * 0.5 is one traversal of the ramp across the span, which is exactly the
   * spatial colour gradient the static banner shipped with.
   *
   * SAFE RANGE 0.125..2.0, measured against the no-worse-than-static gate in
   * CIEDE2000: worst excess over a theme's own static step is 0.47 at 0.5 and
   * stays under 0.62 all the way to 2.0. At 4.0 it hits 8.92 and fails 24
   * themes — that is where the lattice genuinely reads as stripes rather than a
   * wash. Contrast is INDEPENDENT of this knob (it only rescales position along
   * an axis whose floors are proven end to end), so the binding constraint here
   * is spatial busyness, not legibility.
   */
  wavelength: number;
  /**
   * Fraction of the colour ramp the wave actually sweeps, 0..1. BIGGER = MORE
   * COLOUR TRAVEL; 0 pins every cell to the ramp's midpoint (a still, single
   * colour) and 1 uses the full three-stop sweep.
   *
   * CONTRAST-SAFE OVER ITS WHOLE RANGE, and that is a property of the design
   * rather than of the default: intensity only narrows which part of the hue
   * axis gets used, and the contrast gate covers the ENTIRE hue axis on all 62
   * themes. So no value in [0,1] can reach an unsampled, unproven colour.
   * Values outside [0,1] are clamped for the same reason.
   */
  intensity: number;
};

/**
 * Tyler: field originates BOTTOM-RIGHT and travels to TOP-LEFT, so the
 * direction vector points up-and-left. The wordmark keeps its own
 * left-to-right sweep, anchored to the letters rather than the viewport.
 *
 * Tyler, amendment (msg 4e7abbc8): "double the animation speed on the buzz
 * term splash screen". Both periods halved from the original 26s/19s; the
 * "slow" gate's floor moved from 10s to 5s to match.
 */
export const FIELD_WAVE: BannerWaveConfig = {
  direction: { x: -1, y: -1 },
  seconds: 13,
  wavelength: 0.5,
  intensity: 1,
};

export const WORDMARK_WAVE: BannerWaveConfig = {
  // Left-to-right along the mark's own ink span; y is unused for the wordmark,
  // whose axis is the geometry's `t`, not a screen position.
  direction: { x: 1, y: 0 },
  seconds: 9.5,
  wavelength: 0.5,
  intensity: 1,
};

/** Kept as named exports because the "slow" and seam gates assert on them. */
export const FIELD_PERIOD_SECONDS = FIELD_WAVE.seconds;
export const WORDMARK_PERIOD_SECONDS = WORDMARK_WAVE.seconds;

/**
 * Sampling resolution of the `hue` axis.
 *
 * Bucket COUNT is not what makes the gradient smooth — `lookup` interpolates
 * between neighbouring samples, so its output is continuous in hue at any
 * count. I measured the alternative first and it does not work: with
 * nearest-bucket lookup the worst adjacent-sample step on light-plus only fell
 * from 22.1 to 7.0 going from 256 buckets to 2048, because the three-stop ramp
 * has a genuinely steep stretch rather than a quantisation artifact. Eight
 * times the memory for a step still visible on a light theme is the wrong
 * trade; interpolating removes the axis entirely.
 *
 * So this count controls FIDELITY only: how closely the piecewise-linear
 * reconstruction tracks the true curved LCh path. 128 samples holds every
 * shipped theme's deviation from `bannerColor` under the fidelity gate.
 */
export const HUE_BUCKETS = 128;

/**
 * Sampling resolution of the field's radial CONTRAST axis, also interpolated.
 *
 * This axis does not move, so it costs nothing per frame — but it multiplies
 * the fill: the field's `atRatio` pin is a 28-step bisection at ~23us a call
 * against ~0.9us for the wordmark's `lift` path, so the field table is
 * essentially the whole build cost. Interpolation is what lets this stay small
 * enough to fill without stalling the reveal.
 */
export const CONTRAST_BUCKETS = 24;

/**
 * Position on a sampled axis, as a fractional index into `buckets` samples
 * taken at bucket CENTRES. Clamped at both ends so the half-bucket beyond the
 * first and last centres reads as that centre rather than extrapolating.
 */
const axisPosition = (value: number, buckets: number) =>
  Math.min(buckets - 1, Math.max(0, Math.min(1, value) * buckets - 0.5));

/** Wrap into [0,1). Phase is cyclic; a bare modulo leaves negatives negative. */
const wrap01 = (value: number) => value - Math.floor(value);

/**
 * THE SEAM FIX. A cyclic axis cannot drive an open ramp directly.
 *
 * The three-stop ramp is a PATH, not a loop: `ramp(0)` is stop 0 and `ramp(1)`
 * is stop 2, and those two ends are as far apart as the palette gets. Feeding
 * it a wrapping phase means that every cycle, somewhere on screen, adjacent
 * cells straddle the 1 -> 0 wrap and jump the entire width of the palette. On
 * buzz-dark that seam measured 182.5 in sRGB distance (#3dcf55 against
 * #7bb7ff) — a hard colour edge sweeping across the lattice once per period,
 * which is not "slow waves of the theme colours", it is a glitch.
 *
 * The fix is to make the cyclic axis traverse the ramp and come back:
 * 0 -> 1 over the first half of the cycle, 1 -> 0 over the second. The ends
 * are then interior turning points and adjacency holds everywhere, so the loop
 * is seamless by construction rather than by choice of period.
 *
 * THIS DOES NOT REVERSE THE WAVE, and an earlier version of this comment
 * claimed it did. The claim was false and it cost a review round (Wren's #4541
 * gate), so the reasoning is recorded here rather than left to be re-derived.
 *
 * Write the field as `hue(x, phase) = A(G(p(x) + phase))`, where `p` is affine
 * in position (see `projectWave`), `G` is this fold, and `A` is `applyIntensity`
 * (affine, monotone). Phase enters `G` ONLY through the sum `p(x) + phase`, so
 * for a shift `d` along +direction, `p(x + d*u) = p(x) - d*wavelength/span` and
 *
 *     hue(x + d*u, phase + D) = hue(x, phase)   for   d = D * span / wavelength
 *
 * exactly, at every phase, FOR ANY `G` WHATSOEVER. The shape of `G` sets the
 * spatial PROFILE; it cannot touch the direction of MOTION. `d` has the sign of
 * `D`, so travel is uniformly one-way for as long as phase advances, and one
 * full cycle translates the pattern by exactly one spatial period -- continuous
 * BR->TL travel and a seamless wrap at the same time, not a trade between them.
 *
 * The mistake worth not repeating: the fold DOES make a fixed cell's colour
 * oscillate (cell(60,25) reads 0.5 0.7 0.9 0.9 0.7 0.5 0.3 0.1 0.1 0.3 0.5 over
 * one cycle). That is a property of a POINT, not of the pattern -- it is what
 * every travelling wave does to a point it passes over -- and reading it as a
 * direction reversal is what put the false sentence here. `travelDirection` in
 * the test file measures the pattern across the WHOLE cycle, including the
 * alleged turn at phase 0.5 and the wrap, with a time-folded positive control.
 *
 * What the fold does cost is aesthetic, not kinematic: the spatial palette
 * profile is mirrored (A->C->A) rather than circulating (A->B->C->A).
 */
export function cyclicRampPosition(phase: number): number {
  const wrapped = wrap01(phase);
  return wrapped < 0.5 ? wrapped * 2 : 2 - wrapped * 2;
}

/** Cell aspect: cells are 8.4 x 17 CSS px, so one row is ~2.02 columns tall. */
const CELL_ASPECT = 17 / 8.4;

/**
 * Apply `intensity` about the ramp's midpoint. Narrowing rather than scaling
 * keeps the sweep centred, so turning intensity down fades toward the middle
 * colour instead of sliding toward one end.
 */
const applyIntensity = (position: number, intensity: number) =>
  0.5 + (position - 0.5) * Math.max(0, Math.min(1, intensity));

/**
 * THE WAVE AXIS: a projection onto a direction vector in VISUAL space.
 *
 * `phase(cell) = (x*dx + y*dy)/wavelength - t*speed`, per Eva's spec, with two
 * details that are easy to get wrong and invisible in a still frame:
 *
 *  - ASPECT. Row indices are multiplied by ~2.02 before projecting, because a
 *    cell is about twice as tall as it is wide. Without this a (-1,-1) vector
 *    renders as a ~27-degree wave, not the 45 degrees it reads as in source.
 *  - SIGN. The projection is NEGATED so the wave travels TOWARD `direction`.
 *    A crest sits where the projection is constant; as time advances, holding
 *    it constant requires moving along +direction. For the field's (-1,-1)
 *    that is up-and-left, i.e. originating bottom-right as Tyler asked.
 *
 * The normalisation divisor is the projection's own visual span, so
 * `wavelength` means the same thing (ramp traversals per screen-width) at every
 * viewport size and for every direction.
 */
export function projectWave(
  config: BannerWaveConfig,
  column: number,
  row: number,
  columns: number,
  rows: number,
): number {
  const { x: dx, y: dy } = config.direction;
  const length = Math.hypot(dx, dy) || 1;
  const unitX = dx / length;
  const unitY = dy / length;
  const visualY = row * CELL_ASPECT;
  const visualHeight = Math.max(1, rows * CELL_ASPECT);
  const span =
    Math.abs(unitX) * Math.max(1, columns) + Math.abs(unitY) * visualHeight;
  return (-(column * unitX + visualY * unitY) / span) * config.wavelength;
}

/**
 * The field's hue axis: projected position plus phase, folded, then narrowed by
 * intensity.
 */
export function fieldHue(
  config: BannerWaveConfig,
  column: number,
  row: number,
  columns: number,
  rows: number,
  phase: number,
): number {
  return applyIntensity(
    cyclicRampPosition(projectWave(config, column, row, columns, rows) + phase),
    config.intensity,
  );
}

/**
 * The wordmark wave: LEFT TO RIGHT, over the mark's own ink span.
 *
 * The geometry already hands us `t` = 0 at the wordmark's left edge to 1 at its
 * right, so the mark's wave rides that rather than viewport columns: the sweep
 * stays anchored to the letters at every viewport width. SUBTRACTING phase is
 * what makes it travel rightward, opposite to the field.
 */
export function wordmarkHue(
  config: BannerWaveConfig,
  t: number,
  phase: number,
): number {
  const along = config.direction.x >= 0 ? t : 1 - t;
  return applyIntensity(
    cyclicRampPosition(along * config.wavelength - phase),
    config.intensity,
  );
}

/** The pair of wave configs in force. Overridable so the knob-extreme gates can
 *  drive documented settings without mutating module state. */
export type BannerWaves = {
  field: BannerWaveConfig;
  wordmark: BannerWaveConfig;
};
export const DEFAULT_WAVES: BannerWaves = {
  field: FIELD_WAVE,
  wordmark: WORDMARK_WAVE,
};

/**
 * A cell's hue axis, as a closure over everything that is constant for a frame.
 *
 * The split is deliberate: viewport extent, phase and tuning are per-FRAME, and
 * only `(layer, t, column, row)` is per-CELL. Passing the frame-constant half
 * per cell is how this function grew to seven positional arguments and started
 * breaking its own call sites on every signature change. Closing over them also
 * keeps the per-cell path allocation-free, which a coordinates object would not.
 *
 * Bevels return the ramp midpoint — they are not part of either wave, and
 * returning `t` here instead would tie the chassis to the field's motion.
 */
export function makeCellHue(
  columns: number,
  rows: number,
  phase: { field: number; wordmark: number },
  waves: BannerWaves = DEFAULT_WAVES,
): (layer: Layer, t: number, column: number, row: number) => number {
  return (layer, t, column, row) => {
    if (layer === "field")
      return fieldHue(waves.field, column, row, columns, rows, phase.field);
    if (layer === "head") return wordmarkHue(waves.wordmark, t, phase.wordmark);
    return 0;
  };
}

/** Phase pair at a wall-clock time, in seconds. */
export function phaseAt(seconds: number): { field: number; wordmark: number } {
  return {
    field: wrap01(seconds / FIELD_WAVE.seconds),
    wordmark: wrap01(seconds / WORDMARK_WAVE.seconds),
  };
}

/**
 * Phase-INDEPENDENT colour table. This is what makes the animation affordable.
 *
 * Measured on this box, buzz-dark, median of 7: calling `bannerColor` per cell
 * costs 42.7ms for a 112x46 banner (256% of a 60Hz frame) and 203ms at 228x90
 * (1218%). Per-cell-per-frame is not a micro-optimisation problem, it is dead
 * on arrival by 2.6-12x. The cost is concentrated: the field's `atRatio` pin is
 * ~23us/cell against ~0.9us for the wordmark's `lift` path.
 *
 * The table is keyed on SAMPLED (layer, hue, contrast) and nothing else, so it
 * does not depend on phase or on time and fills exactly once per palette.
 * Steady-state frames are a lookup and one lerp.
 */
export type BannerColorTable = Readonly<{
  head: readonly string[];
  field: readonly (readonly string[])[];
  bevelHi: string;
  bevelLo: string;
  lookup: (layer: Layer, t: number, hue: number) => string;
}>;

export function buildBannerColorTable(
  palette: TerminalPalette,
): BannerColorTable {
  const { stops } = bannerStops(palette);
  const centre = (index: number, buckets: number) => (index + 0.5) / buckets;

  const head = Array.from({ length: HUE_BUCKETS }, (_, h) =>
    bannerColor(palette, stops, "head", 0, centre(h, HUE_BUCKETS)),
  );
  const field = Array.from({ length: CONTRAST_BUCKETS }, (_, c) =>
    Array.from({ length: HUE_BUCKETS }, (_, h) =>
      bannerColor(
        palette,
        stops,
        "field",
        centre(c, CONTRAST_BUCKETS),
        centre(h, HUE_BUCKETS),
      ),
    ),
  );
  const bevelHi = bannerColor(palette, stops, "bevel_hi", 0);
  const bevelLo = bannerColor(palette, stops, "bevel_lo", 0);

  // NUMERIC MIRRORS of the tables, parsed once at build time. The lookup path is
  // per-cell-per-frame, and `mix()` re-parses its arguments from hex on every
  // call — worse, `hex2rgb(b)` sits INSIDE its per-channel map, so it parses `b`
  // four times. A bilinear field lookup is three nested mixes, so a single cell
  // cost ~12 string parses plus 3 hex formats. Measured before this change:
  // 13.6ms/frame of pure lookup at 228x90 (80% of the animated frame, 101.6% of a
  // 60Hz budget). Parsing once here makes the steady-state path arithmetic.
  const headRgb = head.map(hex2rgb);
  const fieldRgb = field.map((row) => row.map(hex2rgb));

  // Quantization is LOAD-BEARING, not incidental. `rgb2hex` rounds and clamps
  // each channel to an integer, so the old nested-`mix` path quantized its
  // INTERMEDIATE blend before the outer blend consumed it. Skipping that would
  // change output bytes — and the WCAG floor and dE00 certificates were measured
  // against the old bytes. So this reproduces `rgb2hex`'s rounding exactly at the
  // same point the old path applied it, and byte equality is verified by an
  // oracle over 20.4M lookups x 62 themes rather than assumed.
  const quantize = (value: number) =>
    Math.max(0, Math.min(255, Math.round(value)));
  const mixRgb = (a: readonly number[], b: readonly number[], u: number) => [
    quantize(a[0] + (b[0] - a[0]) * u),
    quantize(a[1] + (b[1] - a[1]) * u),
    quantize(a[2] + (b[2] - a[2]) * u),
  ];
  // Same short-circuits as the string `blend` it replaces: at the ends, return
  // the endpoint untouched rather than round-tripping it.
  const blendRgb = (a: readonly number[], b: readonly number[], u: number) =>
    u <= 0 ? a : u >= 1 ? b : mixRgb(a, b, u);

  const sampleHead = (hue: number) => {
    const position = axisPosition(hue, HUE_BUCKETS);
    const low = Math.floor(position);
    const blended = rgb2hex(
      blendRgb(
        headRgb[low],
        headRgb[Math.min(HUE_BUCKETS - 1, low + 1)],
        position - low,
      ),
    );
    // RE-LIFT. The endpoints are solved to sit exactly ON the 4.5 floor, and
    // contrast is convex in the blend parameter, so the chord between two
    // on-floor samples passes UNDER it — measured 4.4766 on solarized-light at
    // hue 0.2188 with 128 buckets. Small, but 4.4766 < 4.5 is a WCAG failure on
    // shipped pixels, and raising the bucket count only shrinks the dip
    // asymptotically without ever removing it. Lifting the blend is exact.
    // `lift` is a no-op when the blend already clears the floor, which is the
    // overwhelming majority of lookups.
    return lift(blended, palette.background, WORDMARK_FLOOR);
  };

  return {
    head,
    field,
    bevelHi,
    bevelLo,
    // Bilinear on (contrast, hue) for the field, linear on hue for the
    // wordmark. This is what makes the output continuous in hue, so bucket
    // count is a fidelity knob rather than the thing standing between the
    // gradient and visible banding.
    lookup: (layer, t, hue) => {
      switch (layer) {
        case "head":
          return sampleHead(hue);
        case "field": {
          const contrastPosition = axisPosition(t, CONTRAST_BUCKETS);
          const contrastLow = Math.floor(contrastPosition);
          const huePosition = axisPosition(hue, HUE_BUCKETS);
          const hueLow = Math.floor(huePosition);
          const hueHigh = Math.min(HUE_BUCKETS - 1, hueLow + 1);
          const hueFraction = huePosition - hueLow;
          const near = fieldRgb[contrastLow];
          const far = fieldRgb[Math.min(CONTRAST_BUCKETS - 1, contrastLow + 1)];
          return rgb2hex(
            blendRgb(
              blendRgb(near[hueLow], near[hueHigh], hueFraction),
              blendRgb(far[hueLow], far[hueHigh], hueFraction),
              contrastPosition - contrastLow,
            ),
          );
        }
        case "bevel_hi":
          return bevelHi;
        case "bevel_lo":
          return bevelLo;
      }
    },
  };
}
