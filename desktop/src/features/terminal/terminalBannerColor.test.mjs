import assert from "node:assert/strict";
import test from "node:test";

import { extractTerminalPalette } from "../../shared/theme/terminal-palette.ts";
import {
  SYNTAX_THEMES,
  loadThemeData,
} from "../../shared/theme/theme-loader.ts";
import {
  bannerColor,
  bannerStops,
  stopsAreDistinct,
  de00,
  hex2rgb,
  lab,
  ratio,
} from "./terminalBannerColor.ts";

// Gates. contrast 4.5 is WCAG 2.2 SC 1.4.3 (body text); the rest are HOUSE
// thresholds with no external standard, sized by sweep.
const CONTRAST_MIN = 4.5;
const CHROMA_MIN = 16;
const HUE_MIN = 12;
const SRGB_MIN = 45;
const SAMPLES = 101;

const chroma = (h) => Math.hypot(lab(h).a, lab(h).b);
/** Hue-only CIEDE2000: both colours forced to a shared L, so lightness cannot
 *  rescue a pair that is one hue at two brightnesses. */
const hueDist = (a, b) => {
  const p = lab(a),
    q = lab(b),
    L = (p.L + q.L) / 2;
  return de00({ L, a: p.a, b: p.b }, { L, a: q.a, b: q.b });
};
const srgbDist = (a, b) =>
  Math.hypot(...hex2rgb(a).map((v, i) => v - hex2rgb(b)[i]));
const minPair = (s, f) => Math.min(f(s[0], s[1]), f(s[1], s[2]), f(s[0], s[2]));
/** Signed short-arc hue step, in DEGREES. Degrees matter: the reversal
 *  deadband below is calibrated in them. */
const shortArcStep = (a, b) => {
  const p = lab(a),
    q = lab(b);
  let d = (Math.atan2(q.b, q.a) - Math.atan2(p.b, p.a)) * (180 / Math.PI);
  if (d > 180) d -= 360;
  if (d < -180) d += 360;
  return d;
};
// 8-bit output quantization jitters the hue angle by a few tenths of a degree,
// so sub-0.5deg steps are noise, not reversals. Counting them flagged 56 clean
// themes. The gate is the COUNT of real reversals, allowed up to 10% of samples.
const REVERSAL_DEADBAND_DEG = 0.5;

// Production loader + production extractor: the palettes under test are the
// ones the app actually renders, not a fixture that can drift from them.
async function palettes() {
  const out = [];
  for (const name of SYNTAX_THEMES) {
    out.push([name, extractTerminalPalette(await loadThemeData(name))]);
  }
  return out;
}

const THEMES = await palettes();

test("every shipped theme is censused", () => {
  assert.equal(THEMES.length, SYNTAX_THEMES.length);
  assert.equal(THEMES.length, 62);
});

test("wordmark holds contrast and chroma across the whole sweep, not just at the stops", () => {
  // Dense sampling is load-bearing: an RGB lerp between opposing hues crosses
  // the neutral axis, so the fade greys out mid-glyph while all three STOPS
  // stay chromatic. Endpoint-only checks pass that bug. (kills M4)
  const bad = [];
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    for (let i = 0; i < SAMPLES; i++) {
      const c = bannerColor(p, stops, "head", i / (SAMPLES - 1));
      const cr = ratio(c, p.background);
      if (cr < CONTRAST_MIN - 0.005)
        bad.push(`${name} contrast ${cr.toFixed(2)}`);
      if (chroma(c) < CHROMA_MIN)
        bad.push(`${name} chroma ${chroma(c).toFixed(1)}`);
    }
  }
  assert.deepEqual(bad.slice(0, 8), [], `${bad.length} failures`); // kills M1, M2, M10, M11
});

test("themes whose own palette lacks three hues fall back to synthesis", () => {
  // This is where the distinctness gate is OBSERVABLE. Asserting only that the
  // returned stops pass the gate is vacuous — the gate is what produced them,
  // and with it disabled the fallback still yields passing stops. What changes
  // is WHICH themes take the fallback, so that is what gets pinned.
  const synth = THEMES.filter(([, p]) => bannerStops(p).mode === "synth")
    .map(([name]) => name)
    .sort();
  assert.deepEqual(synth, ["min-dark", "vesper"]);
});

test("the three stops are distinct by hue AND by sRGB distance", () => {
  // Conjunction, not either alone: sRGB counts lightness, so white vs #a0a0a0
  // scores 165 while being hue-identical; hue alone would admit stops that
  // differ in angle but barely in appearance.
  const bad = [];
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    const h = minPair(stops, hueDist),
      d = minPair(stops, srgbDist);
    if (h < HUE_MIN) bad.push(`${name} hue ${h.toFixed(1)}`);
    if (d < SRGB_MIN) bad.push(`${name} srgb ${d.toFixed(1)}`);
  }
  assert.deepEqual(bad, []); // kills M3
});

test("each half of the distinctness gate rejects what the other half admits", () => {
  // Tested directly on synthetic triads, NOT via the 62-theme census, because
  // on the shipped themes the two halves mask each other: disabling either
  // alone changes no theme's stops. A census-only test lets a reviewer delete
  // one half and see all green.
  //
  // Hue-blind: three greys. sRGB says "far apart", hue says "same colour".
  assert.equal(stopsAreDistinct(["#ffffff", "#a0a0a0", "#606060"]), false);
  // Distance-blind: muted triad at hue 19.1deg, which CLEARS the hue half,
  // but only 29.7 apart in sRGB — three colours on paper, one smudge on
  // screen. Only the distance half rejects it.
  assert.equal(stopsAreDistinct(["#6a7f8f", "#7f6a8f", "#8f7f6a"]), false);
  // A real shipped triad (dark-plus) passes both.
  assert.equal(stopsAreDistinct(["#569cd6", "#d7ba7d", "#b5cea8"]), true);
});

test("sRGB distance alone cannot gate distinctness", () => {
  // Negative control on the gate itself: if this ever fails, the conjunction
  // has stopped being a conjunction.
  assert.ok(srgbDist("#ffffff", "#a0a0a0") > 60);
  assert.ok(hueDist("#ffffff", "#a0a0a0") < 1);
});

test("the gradient walks the short hue arc and never reverses", () => {
  // Total-variation counting was too noisy to catch a long-arc mutant (8-bit
  // jitter -> false positives), and an off-arc angle tolerance misses it when
  // the two arcs are near-equal (tokyo-night spans 178.3 deg). The invariant
  // that holds: every step moves in the short-arc direction. (kills M5)
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    for (const [lo, hi, a, b] of [
      [0, 0.5, 0, 1],
      [0.5, 1, 1, 2],
    ]) {
      const want = Math.sign(shortArcStep(stops[a], stops[b]));
      if (want === 0) continue;
      let wrong = 0,
        prev = bannerColor(p, stops, "head", lo);
      for (let i = 1; i <= 50; i++) {
        const c = bannerColor(p, stops, "head", lo + ((hi - lo) * i) / 50);
        const step = shortArcStep(prev, c);
        if (Math.abs(step) > REVERSAL_DEADBAND_DEG && Math.sign(step) !== want)
          wrong++;
        prev = c;
      }
      assert.ok(wrong <= 5, `${name}: ${wrong} wrong-direction steps of 50`);
    }
  }
});

test("the field fades monotonically from the box out to the rim", () => {
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    const r = [...Array(SAMPLES)].map((_, i) =>
      ratio(bannerColor(p, stops, "field", i / (SAMPLES - 1)), p.background),
    );
    for (let i = 1; i < r.length; i++) {
      assert.ok(r[i] <= r[i - 1] + 0.02, `${name} non-monotone at ${i}`); // kills M7
    }
    assert.ok(r[0] > r[r.length - 1] + 1, `${name} field does not fade`);
  }
});

test("the bevel keeps a visible light/shadow gap", () => {
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    const hi = ratio(bannerColor(p, stops, "bevel_hi", 0), p.background);
    const lo = ratio(bannerColor(p, stops, "bevel_lo", 0), p.background);
    // ORIENTED, not just separated: a gap assertion is symmetric under
    // swapping the two bevel layers, and a swap changes neither cell count nor
    // draw-call count. Only a colour-valued, directional check sees it.
    assert.ok(
      hi > lo,
      `${name} bevel inverted: hi ${hi.toFixed(2)} <= lo ${lo.toFixed(2)}`,
    );
    assert.ok(hi - lo >= 1.0, `${name} bevel gap ${(hi - lo).toFixed(2)}`);
  }
});

test("every layer returns a colour", () => {
  // A renamed layer silently returned undefined once; this is that regression.
  for (const [name, p] of THEMES.slice(0, 3)) {
    const { stops } = bannerStops(p);
    for (const layer of ["field", "head", "bevel_hi", "bevel_lo"]) {
      assert.match(
        bannerColor(p, stops, layer, 0.5),
        /^#[0-9a-f]{6}$/,
        `${name}/${layer}`,
      );
    }
  }
});

test("contrast is lifted to the floor, never pinned down to it", () => {
  // A floor is not a target: pinning dimmed 30 of 62 themes and cost chroma.
  const spread = THEMES.map(([, p]) => {
    const { stops } = bannerStops(p);
    return Math.max(...stops.map((s) => ratio(s, p.background)));
  });
  assert.ok(Math.max(...spread) > 8, "no theme exceeds the floor => pinned");
});
