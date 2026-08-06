/**
 * Terminal banner colour derivation — the colour half of the banner contract.
 *
 * Geometry (terminalBanner.ts) is pure and carries no colour:
 * grid[row][col] = { char, layer, t }. This module maps (layer, t) -> hex,
 * given the theme's TerminalPalette. Colour is derived in the renderer and
 * regenerated only on viewport or theme change.
 *
 * Why the palette and not the CSS vars: `--primary` is a user preference that
 * collapses to `--foreground` on the Buzz themes (ThemeProvider.tsx), and
 * `--secondary` / `--accent` are assigned the same 6% hover tint
 * (adaptive-theme.ts:241-243). A literal primary/secondary/accent fade is two
 * stops, one of which is the background. The TerminalPalette carries the
 * theme's real hues, so the three stops come from there.
 *
 * Thresholds: contrast 4.5 is WCAG 2.2 SC 1.4.3 (body text). Chroma 16,
 * hue 12 (CIEDE2000 dH', equal-L*) and sRGB 45 are HOUSE thresholds with no
 * external standard — sized by
 * sweep and stable across 8-25 chroma x 8-20 hue.
 *
 * Validated by terminalBannerColor.test.mjs: all 62 shipped themes pass every
 * gate, and 8 mutants of this file are killed by those gates.
 */
import type {
  AnsiColorName,
  TerminalPalette,
} from "../../shared/theme/terminal-palette.ts";

// sRGB -> CIELAB -> CIEDE2000. Instrument for "are two stops visibly distinct".
export const hex2rgb = (h: string) =>
  [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
export const rgb2hex = (r: readonly number[]) =>
  "#" +
  r
    .map((v) =>
      Math.max(0, Math.min(255, Math.round(v)))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("");
const lin = (c: number) => {
  c /= 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};
export const lum = (h: string) => {
  const [r, g, b] = hex2rgb(h);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
};
export const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};
export const mix = (a: string, b: string, t: number) =>
  rgb2hex(hex2rgb(a).map((v, i) => v + (hex2rgb(b)[i] - v) * t));

export function lab(hex: string) {
  const [r, g, b] = hex2rgb(hex).map(lin);
  // sRGB D65 -> XYZ
  let x = r * 0.4124 + g * 0.3576 + b * 0.1805;
  let y = r * 0.2126 + g * 0.7152 + b * 0.0722;
  let z = r * 0.0193 + g * 0.1192 + b * 0.9505;
  x /= 0.95047;
  y /= 1.0;
  z /= 1.08883;
  const f = (t: number) =>
    t > 216 / 24389 ? Math.cbrt(t) : (841 / 108) * t + 4 / 29;
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  return { L: 116 * fy - 16, a: 500 * (fx - fy), b: 200 * (fy - fz) };
}

export function de00(
  p: { L: number; a: number; b: number },
  q: { L: number; a: number; b: number },
) {
  const rad = Math.PI / 180,
    deg = 180 / Math.PI;
  const C1 = Math.hypot(p.a, p.b),
    C2 = Math.hypot(q.a, q.b);
  const Cbar = (C1 + C2) / 2;
  const G = 0.5 * (1 - Math.sqrt(Cbar ** 7 / (Cbar ** 7 + 25 ** 7)));
  const a1p = (1 + G) * p.a,
    a2p = (1 + G) * q.a;
  const C1p = Math.hypot(a1p, p.b),
    C2p = Math.hypot(a2p, q.b);
  const hp = (b: number, a: number) => {
    if (b === 0 && a === 0) return 0;
    const v = Math.atan2(b, a) * deg;
    return v < 0 ? v + 360 : v;
  };
  const h1p = hp(p.b, a1p),
    h2p = hp(q.b, a2p);
  const dLp = q.L - p.L,
    dCp = C2p - C1p;
  let dhp = 0;
  if (C1p * C2p !== 0) {
    dhp = h2p - h1p;
    if (dhp > 180) dhp -= 360;
    else if (dhp < -180) dhp += 360;
  }
  const dHp = 2 * Math.sqrt(C1p * C2p) * Math.sin((dhp / 2) * rad);
  const Lbp = (p.L + q.L) / 2,
    Cbp = (C1p + C2p) / 2;
  let hbp = h1p + h2p;
  if (C1p * C2p !== 0) {
    if (Math.abs(h1p - h2p) > 180) hbp += h1p + h2p < 360 ? 360 : -360;
    hbp /= 2;
  }
  const T =
    1 -
    0.17 * Math.cos((hbp - 30) * rad) +
    0.24 * Math.cos(2 * hbp * rad) +
    0.32 * Math.cos((3 * hbp + 6) * rad) -
    0.2 * Math.cos((4 * hbp - 63) * rad);
  const dTh = 30 * Math.exp(-(((hbp - 275) / 25) ** 2));
  const Rc = 2 * Math.sqrt(Cbp ** 7 / (Cbp ** 7 + 25 ** 7));
  const Sl = 1 + (0.015 * (Lbp - 50) ** 2) / Math.sqrt(20 + (Lbp - 50) ** 2);
  const Sc = 1 + 0.045 * Cbp,
    Sh = 1 + 0.015 * Cbp * T;
  const Rt = -Math.sin(2 * dTh * rad) * Rc;
  return Math.sqrt(
    (dLp / Sl) ** 2 +
      (dCp / Sc) ** 2 +
      (dHp / Sh) ** 2 +
      Rt * (dCp / Sc) * (dHp / Sh),
  );
}

import type { TerminalBannerLayer } from "./terminalBanner.ts";
export type Layer = TerminalBannerLayer;

// WCAG 2.2 SC 1.4.3: 4.5:1 body text. The wordmark is content, not decoration.
const WORDMARK_FLOOR = 4.5;
// Ours, tuning, no standard:
const FIELD_CORE = 2.6; // hex field nearest the box — present, not competing
const FIELD_EDGE = 1.04; // at the rim — all but gone
const BEVEL_HI = 2.9; // lit edge
const BEVEL_LO = 1.55; // shadowed edge; the ratio gap IS the bevel
const CHROMA_MIN = 16; // Lab C*: below this a "hue" is a grey
// UNIT: CIEDE2000 dH' with both colours forced to a shared L*. NOT raw LCh
// hue-angle degrees, which run ~1.5-3x larger on the same stops (red 13.0 vs
// 19.3, kanagawa-dragon 25.1 vs 75.3) because S_H weights by chroma and hue.
// Quoting a degrees figure next to this threshold compares different units.
const HUE_MIN = 12;
const SRGB_MIN = 45; // min pairwise sRGB distance, conjoined with HUE_MIN

/** Euclidean sRGB distance. Guards near-coincident stops that differ in hue
 *  angle but barely in appearance. NOT sufficient alone: it counts lightness,
 *  so white vs #a0a0a0 scores 165 while being hue-identical (hueDist 0.0). */
const srgbDist = (a: string, b: string) =>
  Math.hypot(...hex2rgb(a).map((v, i) => v - hex2rgb(b)[i]));

const chroma = (h: string) => {
  const v = lab(h);
  return Math.hypot(v.a, v.b);
};
/** CIEDE2000 with both colours forced to a shared L — lightness must not
 *  rescue a pair that is really one hue at two brightnesses. */
const hueDist = (a: string, b: string) => {
  const p = lab(a),
    q = lab(b),
    L = (p.L + q.L) / 2;
  return de00({ L, a: p.a, b: p.b }, { L, a: q.a, b: q.b });
};
function away(c: string, bg: string, floor: number) {
  const dir = lum(bg) > 0.5 ? "#000000" : "#ffffff";
  let x = c;
  for (let i = 0; i < 200 && ratio(x, bg) < floor; i++) x = mix(x, dir, 0.02);
  return x;
}
/** Solve for a CONTRAST RATIO, never a fixed mix fraction: one constant
 *  fraction gives a 2.6x appearance spread across themes. */
function atRatio(colour: string, bg: string, target: number) {
  const have = ratio(colour, bg);
  if (have < target) return away(colour, bg, target);
  // Idempotence: re-pinning an already-compliant colour bisects it a hair
  // closer to the ground each time, and every pass costs chroma. Measured on
  // kanagawa-dragon: a second pin dropped a stop from C* 16.1 to 15.5.
  if (have - target < 0.05) return colour;
  let lo = 0,
    hi = 1,
    best = colour;
  for (let i = 0; i < 28; i++) {
    const m = (lo + hi) / 2,
      c = mix(colour, bg, m);
    if (ratio(c, bg) > target) {
      best = c;
      lo = m;
    } else hi = m;
  }
  return best;
}
function lab2hex(L: number, A: number, B: number) {
  const fy = (L + 16) / 116,
    fx = fy + A / 500,
    fz = fy - B / 200;
  const fi = (t: number) =>
    t ** 3 > 216 / 24389 ? t ** 3 : (116 * t - 16) / (24389 / 27);
  const x = fi(fx) * 0.95047,
    y = L > 8 ? ((L + 16) / 116) ** 3 : L / (24389 / 27),
    z = fi(fz) * 1.08883;
  const g = (c: number) => {
    c = Math.max(0, Math.min(1, c));
    return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
  };
  const to = (v: number) =>
    Math.round(g(v) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${to(x * 3.2406 + y * -1.5372 + z * -0.4986)}${to(x * -0.9689 + y * 1.8758 + z * 0.0415)}${to(x * 0.0557 + y * -0.204 + z * 1.057)}`;
}

/** Raise to the floor if short, otherwise leave alone. A floor is a floor,
 *  not a target: `atRatio` also pins DOWN, which dimmed every vivid theme to
 *  exactly 4.5 and cost chroma (tokyo-night's #f7768e sits at 6.46 natively).
 *  Use this wherever the requirement is "at least", and `atRatio` only where a
 *  specific ratio is the design (the field decay and the bevel gap).
 *
 *  Exported because the animation's colour table interpolates BETWEEN solved
 *  samples, and a straight-line blend under a convex contrast curve lands
 *  slightly below both endpoints — measured at 4.4766 against the 4.5 floor on
 *  solarized-light. The blend has to be re-lifted with the same operator that
 *  put the endpoints on the floor in the first place. */
export const lift = (c: string, bg: string, floor: number) =>
  ratio(c, bg) >= floor ? c : away(c, bg, floor);

/**
 * The distinctness gate. Conjunction by design, and each half is tested
 * directly (terminalBannerColor.test.mjs) because on the 62 shipped themes the
 * two halves MASK EACH OTHER: disabling either alone changes no theme's stops,
 * only disabling both does. A census can therefore never justify keeping both.
 *
 * Neither half is sufficient. sRGB distance counts lightness, so white vs
 * #a0a0a0 scores 165 while being one hue. Hue alone admits stops that differ
 * in angle but barely in appearance.
 *
 * Thresholds: hue 12 (CIEDE2000 dH', equal-L*) and sRGB 45 are HOUSE
 * thresholds with no external
 * standard, sized by sweep. The contrast floor 4.5 is WCAG 2.2 SC 1.4.3.
 */
export function stopsAreDistinct(s: [string, string, string]): boolean {
  const min = (f: (a: string, b: string) => number) =>
    Math.min(f(s[0], s[1]), f(s[1], s[2]), f(s[0], s[2]));
  return min(hueDist) >= HUE_MIN && min(srgbDist) >= SRGB_MIN;
}

const HUES = ["red", "green", "yellow", "blue", "magenta", "cyan"] as const;

/** The three wordmark stops: maximally hue-separated, each at the text floor. */
export function bannerStops(p: TerminalPalette): {
  stops: [string, string, string];
  mode: "native" | "synth";
} {
  const bg = p.background;
  const seen = new Set<string>();
  const pool: string[] = [];
  for (const n of HUES)
    for (const k of [n, `bright${n[0].toUpperCase()}${n.slice(1)}`]) {
      const c = lift(p.ansi[k as AnsiColorName], bg, WORDMARK_FLOOR);
      if (!seen.has(c)) {
        seen.add(c);
        pool.push(c);
      }
    }
  const chromatic = pool.filter((c) => chroma(c) >= CHROMA_MIN);
  const pick = (cs: string[]) => {
    let seed = [cs[0], cs[1]],
      best = -1;
    for (let i = 0; i < cs.length; i++)
      for (let j = i + 1; j < cs.length; j++) {
        const d = hueDist(cs[i], cs[j]);
        if (d > best) {
          best = d;
          seed = [cs[i], cs[j]];
        }
      }
    const ch = [...seed];
    let third = cs[0],
      td = -1;
    for (const c of cs) {
      if (ch.includes(c)) continue;
      const d = Math.min(...ch.map((x) => hueDist(x, c)));
      if (d > td) {
        td = d;
        third = c;
      }
    }
    return [...ch, third];
  };
  let stops: string[] | null = null;
  if (chromatic.length >= 3) {
    const s = pick(chromatic);
    // Conjunction. Neither alone is sufficient: sRGB distance counts
    // lightness, so it scores white-vs-grey 165 while they are one hue; and
    // hue alone admits stops that differ in angle but barely in appearance.
    if (stopsAreDistinct(s as [string, string, string])) stops = s;
  }
  let mode: "native" | "synth" = "native";
  if (!stops) {
    // Near-monochrome theme (vesper, min-dark): rotate +/-40 deg in LCh around
    // the theme's own most chromatic colour. It stays the theme's hue family —
    // a restrained theme stays restrained, it just gains a fade.
    mode = "synth";
    const anchor = pool.reduce((a, b) => (chroma(b) > chroma(a) ? b : a));
    const v = lab(anchor),
      C = Math.max(Math.hypot(v.a, v.b), 22),
      h0 = (Math.atan2(v.b, v.a) * 180) / Math.PI;
    stops = [-40, 0, 40].map((d) => {
      const h = ((h0 + d) * Math.PI) / 180;
      return lift(
        lab2hex(v.L, C * Math.cos(h), C * Math.sin(h)),
        bg,
        WORDMARK_FLOOR,
      );
    });
  }
  // order by hue angle so the sweep is monotone across the wordmark
  stops.sort((a, b) => {
    const x = lab(a),
      y = lab(b);
    return Math.atan2(x.b, x.a) - Math.atan2(y.b, y.a);
  });
  return { stops: stops as [string, string, string], mode };
}

/**
 * 3-stop ramp interpolated in CIELCh (polar Lab) along the SHORT hue arc,
 * then re-solved to the target ratio so every sampled point keeps its contrast
 * instead of sagging between stops.
 *
 * Not RGB interpolation: a straight line in RGB between two opposing hues
 * passes through the neutral axis, so the fade greys out mid-glyph. Measured
 * on vitesse-dark, whose stops are all C* >= 24 yet whose RGB midpoint fell to
 * C* 2.3 — three chromatic stops and a grey wordmark. Interpolating the hue
 * ANGLE keeps chroma up across the whole sweep.
 */
function lerpLch(a: string, b: string, u: number) {
  const p = lab(a),
    q = lab(b);
  const [C1, C2] = [Math.hypot(p.a, p.b), Math.hypot(q.a, q.b)];
  const h1 = Math.atan2(p.b, p.a),
    h2 = Math.atan2(q.b, q.a);
  let dh = h2 - h1; // short arc
  if (dh > Math.PI) dh -= 2 * Math.PI;
  if (dh < -Math.PI) dh += 2 * Math.PI;
  const L = p.L + (q.L - p.L) * u;
  const C = C1 + (C2 - C1) * u;
  const h = h1 + dh * u;
  return lab2hex(L, C * Math.cos(h), C * Math.sin(h));
}
function ramp(
  stops: [string, string, string],
  t: number,
  bg: string,
  target: number,
  pin = true,
) {
  const u = Math.max(0, Math.min(1, t));
  const c =
    u < 0.5
      ? lerpLch(stops[0], stops[1], u * 2)
      : lerpLch(stops[1], stops[2], (u - 0.5) * 2);
  return pin ? atRatio(c, bg, target) : lift(c, bg, target);
}

/**
 * `t` and `hue` are TWO AXES, and on the field they are not the same axis.
 *
 * A cell's `t` from the geometry is radial: `ray * ray`, a cast outward from
 * the chassis. It sets the field's CONTRAST decay (2.60 at the box to 1.04 at
 * the rim) and it also used to set the hue position, because a static banner
 * has no reason to tell them apart. An animated wave does: driving hue from
 * the radial axis makes the colour travel outward from the box like a ripple,
 * and driving the contrast target from a moving axis makes the rim pulse
 * bright and breaks the monotonic fade the field is specced on.
 *
 * So `hue` selects the position along the three-stop ramp and `t` keeps the
 * contrast decay. `hue` DEFAULTS TO `t`, which is exactly the coupled
 * behaviour the static banner shipped with — callers that omit it get the
 * shipped pixels, byte for byte.
 */
export function bannerColor(
  p: TerminalPalette,
  stops: [string, string, string],
  layer: Layer,
  t: number,
  hue: number = t,
): string {
  const bg = p.background;
  switch (layer) {
    case "head":
      // Tyler: "fill the box with buzz term in a three color fade".
      return ramp(stops, hue, bg, WORDMARK_FLOOR, false);
    case "field":
      // Tyler: "flow to the edges, fading out" + "colour shift centre to edge".
      // Same three hues, so the field and the mark are one palette; the fade is
      // in CONTRAST, from 2.60 at the box to 1.04 at the rim.
      return ramp(
        stops,
        hue,
        bg,
        FIELD_CORE + (FIELD_EDGE - FIELD_CORE) * Math.max(0, Math.min(1, t)),
      );
    case "bevel_hi":
      return atRatio(p.foreground, bg, BEVEL_HI);
    case "bevel_lo":
      return atRatio(p.foreground, bg, BEVEL_LO);
  }
}
