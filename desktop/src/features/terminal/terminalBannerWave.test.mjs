import assert from "node:assert/strict";
import test from "node:test";

import { extractTerminalPalette } from "../../shared/theme/terminal-palette.ts";
import {
  SYNTAX_THEMES,
  loadThemeData,
} from "../../shared/theme/theme-loader.ts";
import { buildTerminalBanner } from "./terminalBanner.ts";
import {
  bannerColor,
  bannerStops,
  de00,
  lab,
  ratio,
} from "./terminalBannerColor.ts";
import { paintTerminalBanner } from "./terminalBannerPainter.ts";
import {
  CONTRAST_BUCKETS,
  FIELD_PERIOD_SECONDS,
  FIELD_WAVE,
  HUE_BUCKETS,
  WORDMARK_PERIOD_SECONDS,
  WORDMARK_WAVE,
  buildBannerColorTable,
  cyclicRampPosition,
  makeCellHue,
  phaseAt,
} from "./terminalBannerWave.ts";

// WCAG 2.2 SC 1.4.3 for the wordmark; the field/bevel figures are the house
// thresholds terminalBannerColor.ts is specced on.
const WORDMARK_FLOOR = 4.5;
const FIELD_CORE = 2.6;
const FIELD_EDGE = 1.04;

const COLUMNS = 112;
const ROWS = 46;
const ASPECT = 17 / 8.4;

/** Phases sampled by every per-frame gate. Deliberately includes 0.5, the fold
 *  turn, and values either side of it: the turning point is where a fold bug
 *  would show and an evenly-spaced sweep can step right over it. */
const PHASE_SWEEP = [
  0, 0.07, 0.13, 0.21, 0.29, 0.37, 0.44, 0.49, 0.5, 0.51, 0.58, 0.66, 0.73,
  0.81, 0.89, 0.95,
];

/**
 * Every pair of cells that are NEIGHBOURS ON SCREEN and share a layer, in both
 * axes, with their grid coordinates. This is the display's sampling rate — the
 * only rate at which a "banding" claim means anything.
 *
 * Cross-layer pairs are excluded on purpose: the field/wordmark boundary is a
 * designed contrast edge, not a gradient, and including it would swamp the
 * measurement with an intended step.
 */
function adjacentSameLayerPairs(banner) {
  const rows = banner.cells;
  const usable = (cell) => cell && cell.char !== " " && cell.layer;
  const pairs = [];
  for (const [y, cells] of rows.entries())
    for (let x = 1; x < cells.length; x += 1) {
      const a = cells[x - 1];
      const b = cells[x];
      if (!usable(a) || !usable(b) || a.layer !== b.layer) continue;
      pairs.push({ a, b, ax: x - 1, ay: y, bx: x, by: y });
    }
  for (let y = 1; y < rows.length; y += 1)
    for (let x = 0; x < rows[y].length; x += 1) {
      const a = rows[y - 1][x];
      const b = rows[y][x];
      if (!usable(a) || !usable(b) || a.layer !== b.layer) continue;
      pairs.push({ a, b, ax: x, ay: y - 1, bx: x, by: y });
    }
  return pairs;
}

async function palettes() {
  const out = [];
  for (const name of SYNTAX_THEMES) {
    out.push([name, extractTerminalPalette(await loadThemeData(name))]);
  }
  return out;
}
const THEMES = await palettes();

/**
 * PERCEPTUAL colour distance, CIEDE2000 — the metric every smoothness gate below
 * uses, and the same one terminalBannerColor.ts already uses for its stop
 * distinctness gate.
 *
 * Not interchangeable with sRGB distance, and the difference decided a verdict
 * here: euclidean sRGB scores a hue rotation at constant lightness as harshly as
 * a brightness cliff, and this animation is almost entirely hue rotation. See the
 * adjacent-cell gate for the measured 20-of-62 vs 2-of-62 reversal.
 */
const perceptualStep = (a, b) => de00(lab(a), lab(b));

/** One just-noticeable difference in dE00. The conventional threshold for "a
 *  careful observer can tell these apart at all". */
const JND = 1.0;

function recordingCanvas() {
  const draws = [];
  const context = {
    _fillStyle: "",
    clearRect() {},
    fillRect() {},
    fillText(...args) {
      draws.push({ args, color: this._fillStyle });
    },
    font: "",
    set fillStyle(value) {
      this._fillStyle = value;
    },
    get fillStyle() {
      return this._fillStyle;
    },
    setTransform() {},
    textBaseline: "",
  };
  return {
    canvas: {
      height: 0,
      width: 0,
      getBoundingClientRect: () => ({ height: 600, width: 1000 }),
      getContext: () => context,
    },
    draws,
  };
}

/**
 * THE REDUCED-MOTION CONTRACT, as byte equality against an INDEPENDENT oracle.
 *
 * "prefers-reduced-motion = today's static banner" is easy to satisfy wrongly: a
 * paused animation also stops moving, but it parks on whatever phase it halted
 * at, which is NOT the shipped image.
 *
 * MY FIRST VERSION OF THIS GATE WAS VACUOUS, and a mutant proved it. It called
 * `paintTerminalBanner` with no motion twice and compared the two results —
 * which is a tautology: the same function on the same input. Mutating the static
 * path to pass a hue argument (`bannerColor(..., t, 0.37)`) changed BOTH sides
 * identically and the gate stayed green, so the single most important promise in
 * this lane was being checked by an assertion that could not fail.
 *
 * The fix is an oracle the painter does not participate in: walk the banner
 * geometry here in the test and compute each cell's colour with the shipped
 * four-argument `bannerColor` call directly. Now "the static path is byte-identical
 * to the shipped banner" is a claim about two independently derived things.
 */
test("reduced motion paints the shipped static banner, byte for byte", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  assert.ok(banner);
  for (const [name, p] of THEMES.slice(0, 6)) {
    const painted = recordingCanvas();
    assert.equal(paintTerminalBanner(painted.canvas, banner, p, 2), true);

    // The oracle: the shipped colour call, driven straight off the geometry,
    // with no reference to the painter's own colour resolution.
    const { stops } = bannerStops(p);
    const expected = [];
    for (const cells of banner.cells)
      for (const cell of cells) {
        if (cell.char === " " || !cell.layer) continue;
        expected.push(bannerColor(p, stops, cell.layer, cell.t));
      }

    assert.ok(expected.length > 1000, `${name} drew nothing to compare`);
    assert.deepEqual(
      painted.draws.map((draw) => draw.color),
      expected,
      `${name} static path diverged from the shipped colour`,
    );
  }
});

/**
 * The static path must NOT be reachable by handing it a zero phase — proof that
 * the two arms are genuinely different code paths and that this suite's
 * equality test above is not vacuous. If an animated frame at phase 0 already
 * equalled the static banner, the byte-equality test would pass even with the
 * reduced-motion branch deleted.
 */
test("an animated frame at phase zero is NOT the static banner", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  const still = recordingCanvas();
  const moving = recordingCanvas();
  paintTerminalBanner(still.canvas, banner, p, 2);
  paintTerminalBanner(moving.canvas, banner, p, 2, {
    phase: { field: 0, wordmark: 0 },
    table,
  });
  assert.equal(still.draws.length, moving.draws.length);
  const differing = moving.draws.filter(
    (draw, index) => draw.color !== still.draws[index].color,
  ).length;
  assert.ok(
    differing > moving.draws.length / 4,
    `only ${differing} of ${moving.draws.length} draws differ`,
  );
});

/**
 * DIRECTION, OVER THE WHOLE CYCLE. Eva's spec: the field originates
 * BOTTOM-RIGHT and travels to TOP-LEFT; the wordmark stays left-to-right. A
 * sign error is invisible in any still frame, which is why this measures
 * movement over time rather than a property of one frame.
 *
 * THIS GATE USED TO SAMPLE ONLY 0.1 -> 0.2, justified by a comment on
 * `cyclicRampPosition` claiming the wave reversed each half-cycle. That claim
 * was false (see the proof there), and the narrow sampling it licensed is what
 * let a reviewer read the code as back-and-forth motion. The honest version
 * sweeps the ENTIRE cycle, including the alleged turn at phase 0.5 and the
 * 1 -> 0 wrap — the exact two places the old gate did not look.
 *
 * Two instruments, because a crest tracker alone can hop between the level
 * set's two branches and fake a reversal:
 *
 *  A. RIGID-TRANSLATION FIT. If the pattern translates, there is a shift `d`
 *     along +direction with hue(x + d*u, phase + D) == hue(x, phase). Fit `d`
 *     per phase and require ONE SIGN and a constant magnitude across the cycle.
 *     Bounded inside one spatial period: a wider search aliases onto the twin
 *     shift `d + period` and reports a wrong (though same-signed) magnitude.
 *  B. BRANCH-CONTINUOUS CREST TRACKING. Follow the crossing nearest the
 *     previous one — that is what "a crest" means — and require monotone travel
 *     over 200 steps spanning the full cycle.
 *
 * CONTROLS, so the negative is worth something (both must be CAUGHT):
 *   POS-1 phase folded in TIME. This is what "reverses every half-cycle"
 *         actually looks like, and it is the mutant the false comment
 *         described. The shipped spatial fold is not it.
 *   POS-2 negated phase: uniform travel the WRONG way, i.e. a sign error.
 */

/** Visual-space unit vector and one-spatial-period pitch, along a FIXED axis.
 *
 *  Anchored to the SPEC's axis, never to `FIELD_WAVE.direction`: a gate that
 *  derives its own measurement axis from the value under test rotates with a
 *  direction bug and reports success. Measured — flipping the default to
 *  `{x:1,y:1}` survived this gate until the axis was hardcoded here. The config
 *  is checked against this axis separately, below. */
const EXPECTED_FIELD_AXIS = { x: -Math.SQRT1_2, y: -Math.SQRT1_2 };
const fieldGeometry = (columns, rows) => {
  const { x: unitX, y: unitY } = EXPECTED_FIELD_AXIS;
  const span =
    Math.abs(unitX) * Math.max(1, columns) +
    Math.abs(unitY) * Math.max(1, rows * ASPECT);
  return { unitX, unitY, period: span / FIELD_WAVE.wavelength };
};

/**
 * Fit the spatial shift along +direction that undoes a phase advance of
 * `delta`. Returns the shift and its residual; a rigid translation fits with a
 * residual at rounding level, so a large residual means "not a translation"
 * rather than "translated by this much".
 */
const fitTranslation = (fieldAt, phase, delta, geometry, bound) => {
  const samples = [];
  for (let row = 8; row < 42; row += 3)
    for (let column = 8; column < 112; column += 3) samples.push([column, row]);
  const score = (d) => {
    let sum = 0;
    for (const [column, row] of samples) {
      const here = fieldAt(column, row, phase);
      const there = fieldAt(
        column + geometry.unitX * d,
        row + (geometry.unitY * d) / ASPECT,
        phase + delta,
      );
      sum += (here - there) ** 2;
    }
    return Math.sqrt(sum / samples.length);
  };
  let best = 0;
  let bestResidual = Number.POSITIVE_INFINITY;
  for (let d = -bound; d <= bound; d += 0.5) {
    const residual = score(d);
    if (residual < bestResidual) {
      bestResidual = residual;
      best = d;
    }
  }
  for (let d = best - 0.5; d <= best + 0.5; d += 0.01) {
    const residual = score(d);
    if (residual < bestResidual) {
      bestResidual = residual;
      best = d;
    }
  }
  return { shift: best, residual: bestResidual };
};

/** Track one crest along the direction axis by continuity, full cycle. */
const trackCrest = (fieldAt, geometry, columns, rows, steps) => {
  const target = 0.5;
  const at = (s, phase) =>
    fieldAt(
      columns / 2 + geometry.unitX * s,
      rows / 2 + (geometry.unitY * s) / ASPECT,
      phase,
    );
  const nearest = (phase, from) => {
    let best = from;
    let bestKey = Number.POSITIVE_INFINITY;
    for (let s = from - 12; s <= from + 12; s += 0.02) {
      // Prefer the closest match to `target`, tie-broken toward `from` so the
      // tracker stays on ONE branch instead of jumping to an equal crossing.
      const key =
        Math.abs(at(s, phase) - target) * 1000 + Math.abs(s - from) * 1e-6;
      if (key < bestKey) {
        bestKey = key;
        best = s;
      }
    }
    return best;
  };
  let position = nearest(0, 0);
  const start = position;
  let negative = 0;
  let positive = 0;
  for (let index = 1; index <= steps; index += 1) {
    const next = nearest(index / steps, position);
    const step = next - position;
    if (step < -1e-6) negative += 1;
    if (step > 1e-6) positive += 1;
    position = next;
  }
  return { net: position - start, negative, positive };
};

/** The three arms: shipped, plus the two reversal/sign controls. `transform`
 *  maps the animation clock onto the phase actually handed to the module, so
 *  the controls exercise the REAL composition instead of a reimplementation. */
const TRAVEL_ARMS = [
  { name: "shipped", transform: (phase) => phase, oneWay: true },
  {
    name: "POS-1 time-folded phase (a real reversal)",
    transform: (phase) => {
      const wrapped = phase - Math.floor(phase);
      return wrapped < 0.5 ? wrapped * 2 : 2 - wrapped * 2;
    },
    oneWay: false,
  },
  {
    name: "POS-2 negated phase (sign error)",
    transform: (phase) => -phase,
    oneWay: false,
  },
];

/** Verdicts for one arm, so the assertions and the controls share an oracle.
 *
 *  Phase is obtained through the REAL clock path (`phaseAt(seconds)`), not by
 *  handing `makeCellHue` a phase directly. Calling the hue factory with a
 *  hand-made phase tests the wave functions in isolation and is blind to
 *  anything `phaseAt` does — a time-fold injected there survived this gate
 *  until it was routed this way. Same composition gap as M9: isolation gates
 *  compose only if something tests the composition. `transform` perturbs the
 *  CLOCK, so every arm still runs the whole shipped chain. */
const travelVerdict = (transform) => {
  const columns = 120;
  const rows = 50;
  const geometry = fieldGeometry(columns, rows);
  // Phase advances by 0.01 of the FIELD cycle per unit of `phase` here, via the
  // real seconds->phase conversion, so `phaseAt`'s wrapping is in the loop.
  const hueFor = (phase) => {
    const clock = transform(phase) * FIELD_PERIOD_SECONDS;
    return makeCellHue(columns, rows, phaseAt(clock));
  };
  const fieldAt = (column, row, phase) =>
    hueFor(phase)("field", 0.5, column, row);
  const headAt = (t, phase) => hueFor(phase)("head", t, 0, 0);

  // A: fits across the full cycle, spanning the fold turn and the wrap.
  const fits = [
    0, 0.1, 0.2, 0.3, 0.4, 0.45, 0.49, 0.5, 0.51, 0.55, 0.6, 0.7, 0.8, 0.9,
    0.95, 0.99,
  ].map((phase) => ({
    phase,
    ...fitTranslation(fieldAt, phase, 0.01, geometry, geometry.period / 2),
  }));
  const shiftSigns = new Set(
    fits.map((fit) => Math.sign(+fit.shift.toFixed(2))),
  );
  const crest = trackCrest(fieldAt, geometry, columns, rows, 200);

  // Wordmark: crest position in `t`, unwrapped, across the full cycle.
  const tOfCrest = (phase) => {
    let best = 0;
    let bestError = Number.POSITIVE_INFINITY;
    for (let step = 0; step <= 200; step += 1) {
      const t = step / 200;
      const error = Math.abs(headAt(t, phase) - 0.5);
      if (error < bestError) {
        bestError = error;
        best = t;
      }
    }
    return best;
  };
  let headBackwards = 0;
  let headForwards = 0;
  let previous = tOfCrest(0);
  for (let index = 1; index <= 60; index += 1) {
    const current = tOfCrest(index / 60);
    let step = current - previous;
    // The crest leaves the right edge and re-enters at the left: unwrap into
    // (-0.5, 0.5] so a re-entry is not mistaken for backward travel.
    if (step > 0.5) step -= 1;
    if (step < -0.5) step += 1;
    if (step < -1e-9) headBackwards += 1;
    if (step > 1e-9) headForwards += 1;
    previous = current;
  }

  return { fits, shiftSigns, crest, headBackwards, headForwards, geometry };
};

test("the field travels toward top-left and the wordmark left to right, across the WHOLE cycle", () => {
  const { fits, shiftSigns, crest, headBackwards, headForwards, geometry } =
    travelVerdict((phase) => phase);

  // The config must agree with the axis this gate measures along. Hardcoding
  // the axis is what gives the gate teeth against a direction flip, but it also
  // means a config change would silently stop being measured — so assert the
  // two match rather than trusting them to.
  const configLength = Math.hypot(
    FIELD_WAVE.direction.x,
    FIELD_WAVE.direction.y,
  );
  assert.ok(
    Math.abs(FIELD_WAVE.direction.x / configLength - EXPECTED_FIELD_AXIS.x) <
      1e-9 &&
      Math.abs(FIELD_WAVE.direction.y / configLength - EXPECTED_FIELD_AXIS.y) <
        1e-9,
    `FIELD_WAVE.direction ${JSON.stringify(FIELD_WAVE.direction)} is not the ` +
      "spec's bottom-right -> top-left axis this gate measures along",
  );

  // A. One sign, constant magnitude, translation-quality residual.
  assert.deepEqual(
    [...shiftSigns],
    [1],
    `field shift changed sign across the cycle: ${fits
      .map((fit) => `${fit.phase}:${fit.shift.toFixed(2)}`)
      .join(" ")}`,
  );
  const shifts = fits.map((fit) => fit.shift);
  const spread = Math.max(...shifts) - Math.min(...shifts);
  assert.ok(
    spread < 0.05,
    `field velocity is not constant across the cycle: spread ${spread.toFixed(3)}px`,
  );
  const worstResidual = Math.max(...fits.map((fit) => fit.residual));
  assert.ok(
    worstResidual < 1e-3,
    `field is not a rigid translation: worst residual ${worstResidual.toExponential(2)}`,
  );
  // Magnitude matches the closed form d = D * span / wavelength, so the gate
  // pins the aspect correction too: a wrong ASPECT changes `span`.
  const predicted = 0.01 * geometry.period;
  assert.ok(
    Math.abs(shifts[0] - predicted) < 0.05,
    `shift ${shifts[0].toFixed(3)}px disagrees with closed form ${predicted.toFixed(3)}px`,
  );

  // B. Monotone crest travel over the full cycle, and exactly one spatial
  // period of net travel — continuous travel AND a seamless wrap together.
  assert.equal(
    crest.negative,
    0,
    `field crest moved backward on ${crest.negative}/200 steps`,
  );
  assert.ok(crest.positive > 190, `field crest stalled: ${crest.positive}/200`);
  assert.ok(
    Math.abs(crest.net - geometry.period) < geometry.period * 0.05,
    `net travel ${crest.net.toFixed(1)}px is not one spatial period (${geometry.period.toFixed(1)}px)`,
  );

  // Wordmark: strictly rightward over the whole cycle, wrap unwrapped.
  assert.equal(
    headBackwards,
    0,
    `wordmark crest moved leftward on ${headBackwards}/60 steps`,
  );
  assert.ok(headForwards > 55, `wordmark crest stalled: ${headForwards}/60`);
});

/**
 * THE GATE'S OWN NEGATIVE CONTROLS. A direction gate that a reversal can
 * survive is worthless, and the previous one could: POS-1 is precisely the
 * motion the old comment claimed the code had, and the old 0.1->0.2 sampling
 * could not see it. Both controls must be CAUGHT by the assertions above.
 */
test("the travel gate catches a real reversal and a sign error", () => {
  for (const arm of TRAVEL_ARMS.filter((candidate) => !candidate.oneWay)) {
    const { shiftSigns, crest, headBackwards } = travelVerdict(arm.transform);
    const caught =
      shiftSigns.size > 1 ||
      !shiftSigns.has(1) ||
      crest.negative > 0 ||
      headBackwards > 0;
    assert.ok(
      caught,
      `${arm.name} was NOT caught: signs {${[...shiftSigns].join(",")}}, ` +
        `crest negative ${crest.negative}, wordmark backward ${headBackwards}`,
    );
  }

  // And the shipped arm must pass the same oracle, or "caught" above is just
  // an instrument that fails everything.
  const shipped = travelVerdict((phase) => phase);
  assert.deepEqual([...shipped.shiftSigns], [1]);
  assert.equal(shipped.crest.negative, 0);
  assert.equal(shipped.headBackwards, 0);
});

/** The two waves must not be one wave. Same period would read as a single
 *  sweep bouncing, and equal periods is the likeliest careless edit. */
test("the two waves have different periods and neither is instant", () => {
  assert.notEqual(FIELD_PERIOD_SECONDS, WORDMARK_PERIOD_SECONDS);
  // 'Slow' means slow. Ambience, not a screensaver: a full traversal must take
  // more than five seconds on both waves. (Floor halved from 10s with Tyler's
  // "double the animation speed" amendment, msg 4e7abbc8.)
  assert.ok(FIELD_PERIOD_SECONDS > 5);
  assert.ok(WORDMARK_PERIOD_SECONDS > 5);
});

test("phase wraps into [0,1) and advances monotonically within a cycle", () => {
  for (const seconds of [0, 1, 7.5, 25.9, 26, 260.4, 1e5]) {
    const { field, wordmark } = phaseAt(seconds);
    assert.ok(field >= 0 && field < 1, `field phase ${field}`);
    assert.ok(wordmark >= 0 && wordmark < 1, `wordmark phase ${wordmark}`);
  }
  // A full period returns to the start: the loop must be seamless, or the
  // banner visibly jumps once every FIELD_PERIOD_SECONDS.
  assert.ok(Math.abs(phaseAt(FIELD_PERIOD_SECONDS).field - 0) < 1e-9);
  assert.ok(Math.abs(phaseAt(WORDMARK_PERIOD_SECONDS).wordmark - 0) < 1e-9);
});

/** The chassis is not part of either wave. */
test("bevels are phase-invariant", () => {
  for (const phase of [0, 0.13, 0.5, 0.87]) {
    const hue = makeCellHue(COLUMNS, ROWS, { field: phase, wordmark: phase });
    for (const layer of ["bevel_hi", "bevel_lo"]) {
      assert.equal(hue(layer, 0, 40, 20), 0);
    }
  }
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  const hi = new Set();
  const lo = new Set();
  for (const phase of [0, 0.2, 0.4, 0.6, 0.8]) {
    hi.add(table.lookup("bevel_hi", 0, phase));
    lo.add(table.lookup("bevel_lo", 0, phase));
  }
  assert.equal(hi.size, 1);
  assert.equal(lo.size, 1);
});

/**
 * CONTRAST FLOORS ON EVERY FRAME, measured on the QUANTISED TABLE OUTPUT.
 *
 * Gating the ideal `bannerColor` function would be the wrong instrument: the
 * pixels that ship come from the LUT, and a bucket boundary is exactly where a
 * floor gets lost. This walks every table entry — which is every colour the
 * animation can ever produce at any phase, since the table is
 * phase-independent — on all 62 shipped themes.
 */
test("every colour the animation can produce holds its contrast floor", () => {
  const bad = [];
  for (const [name, p] of THEMES) {
    const table = buildBannerColorTable(p);
    const bg = p.background;
    for (const colour of table.head) {
      const cr = ratio(colour, bg);
      if (cr < WORDMARK_FLOOR - 0.005)
        bad.push(`${name} wordmark ${cr.toFixed(2)}`);
    }
    for (const [contrastIndex, row] of table.field.entries()) {
      // The field's specced envelope: it decays from FIELD_CORE at the box to
      // FIELD_EDGE at the rim, so a bound that holds at every bucket is the
      // envelope itself, not a single number.
      const t = (contrastIndex + 0.5) / CONTRAST_BUCKETS;
      const target = FIELD_CORE + (FIELD_EDGE - FIELD_CORE) * t;
      for (const colour of row) {
        const cr = ratio(colour, bg);
        if (cr < FIELD_EDGE - 0.02 || cr > FIELD_CORE + 0.25)
          bad.push(`${name} field ${cr.toFixed(2)} outside envelope`);
        if (Math.abs(cr - target) > 0.35)
          bad.push(
            `${name} field bucket ${contrastIndex} ${cr.toFixed(2)} vs target ${target.toFixed(2)}`,
          );
      }
    }
  }
  assert.deepEqual(bad.slice(0, 8), [], `${bad.length} failures`);
});

/**
 * The field's radial contrast decay must survive the decoupling. This is the
 * invariant that BREAKS if hue and contrast are driven off one axis: animate
 * the coupled `t` and the rim pulses bright. Checked at several phases, since
 * a phase-independent monotonicity is the whole point of the split.
 */
test("the field still fades monotonically outward at every phase", () => {
  for (const [name, p] of THEMES) {
    const table = buildBannerColorTable(p);
    for (const hue of [0, 0.17, 0.33, 0.5, 0.71, 0.94]) {
      let previous = Number.POSITIVE_INFINITY;
      for (let index = 0; index < CONTRAST_BUCKETS; index += 1) {
        const t = (index + 0.5) / CONTRAST_BUCKETS;
        const cr = ratio(table.lookup("field", t, hue), p.background);
        assert.ok(
          cr <= previous + 0.02,
          `${name} non-monotone at bucket ${index} hue ${hue}`,
        );
        previous = cr;
      }
    }
  }
});

/**
 * BANDING, AT THE DISPLAY'S OWN SAMPLING RATE, IN A PERCEPTUAL METRIC, and
 * benchmarked against the shipped static banner rather than an absolute number.
 *
 * THREE CORRECTIONS TO MY OWN EARLIER VERSIONS OF THIS GATE, each measured:
 *
 * 1. WRONG SAMPLING RATE. It first compared ADJACENT HUE BUCKETS, steps of
 *    1/128 in hue. Nothing on screen samples the axis that finely — adjacent
 *    field cells are ~1/112 apart in projected position and one 60Hz frame
 *    advances the phase by 1/(13*60). It was reporting an inherited property of
 *    the ramp's steep stretch, and failed on 24 themes while the pixels were fine.
 *
 * 2. WRONG THRESHOLD. An absolute 8-unit bound was never a standard the shipped
 *    product meets: the STATIC banner already steps 66.3 sRGB between adjacent
 *    cells on light-plus. Hence per-theme and relative — no worse than that
 *    theme's own static worst.
 *
 * 3. WRONG METRIC, and this one reversed the verdict. Euclidean sRGB distance
 *    counts a hue swing at constant lightness the same as a brightness cliff,
 *    and this animation is almost entirely the former. On raw sRGB, 20 of 62
 *    themes scored "worse than static" (max 47.1 vs 68.9); on CIEDE2000 — the
 *    perceptual metric this module already uses for its distinctness gate — it
 *    is 2 of 62, max 13.03 against a static 28.34. The eye is the thing being
 *    gated, so dE00 is the instrument.
 *
 * Tolerance is 1.0 dE00, one just-noticeable difference, over the theme's own
 * static worst. CALIBRATED, not guessed: at the default wavelength the worst
 * excess across 62 themes is 0.47, it stays under 0.62 through wavelength 2.0,
 * and at wavelength 4.0 it jumps to 8.92 and fails 24 themes. The gate
 * discriminates a real stripe regression from measurement noise.
 *
 * Both axes are walked because the wave is now diagonal: aspect correction makes
 * the vertical gradient comparable to the horizontal, so a rows-only gate would
 * miss half the field.
 */
test("adjacent-cell colour steps are no worse than the shipped static banner", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const pairs = adjacentSameLayerPairs(banner);
  assert.ok(pairs.length > 2000, `only ${pairs.length} adjacent pairs`);

  const offenders = [];
  for (const [name, p] of THEMES) {
    const { stops } = bannerStops(p);
    const table = buildBannerColorTable(p);
    const staticColour = (cell) => bannerColor(p, stops, cell.layer, cell.t);
    let staticWorst = 0;
    for (const { a, b } of pairs)
      staticWorst = Math.max(
        staticWorst,
        perceptualStep(staticColour(a), staticColour(b)),
      );

    let animatedWorst = 0;
    for (const phase of PHASE_SWEEP) {
      const hue = makeCellHue(COLUMNS, ROWS, { field: phase, wordmark: phase });
      const colour = (cell, column, row) =>
        table.lookup(cell.layer, cell.t, hue(cell.layer, cell.t, column, row));
      for (const { a, b, ax, ay, bx, by } of pairs)
        animatedWorst = Math.max(
          animatedWorst,
          perceptualStep(colour(a, ax, ay), colour(b, bx, by)),
        );
    }
    if (animatedWorst > staticWorst + JND)
      offenders.push(
        `${name} animated ${animatedWorst.toFixed(1)} > static ${staticWorst.toFixed(1)}`,
      );
  }
  assert.deepEqual(
    offenders.slice(0, 6),
    [],
    `${offenders.length} themes worse than static`,
  );
});

/**
 * The TEMPORAL half of the same question: how far a single cell's colour can move
 * in one 60Hz frame. Spatial smoothness does not imply temporal smoothness — a
 * wave could be a smooth gradient that strobes.
 *
 * Bound is 3 dE00 per frame, three JNDs. Measured worst on the defaults is 2.071
 * across all 62 themes, and a cell needs many frames to cross the ramp, so this
 * is a drift rather than a flicker.
 */
test("no cell's colour moves more than 3 dE00 in one 60Hz frame", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const pairs = adjacentSameLayerPairs(banner);
  const sampled = pairs.filter((_, index) => index % 17 === 0);
  const frame = 1 / 60;
  const offenders = [];
  for (const [name, p] of THEMES) {
    const table = buildBannerColorTable(p);
    let worst = 0;
    for (let step = 0; step < 60; step += 1) {
      const seconds = (step / 60) * FIELD_PERIOD_SECONDS;
      const before = makeCellHue(COLUMNS, ROWS, phaseAt(seconds));
      const after = makeCellHue(COLUMNS, ROWS, phaseAt(seconds + frame));
      for (const { a, ax, ay } of sampled)
        worst = Math.max(
          worst,
          perceptualStep(
            table.lookup(a.layer, a.t, before(a.layer, a.t, ax, ay)),
            table.lookup(a.layer, a.t, after(a.layer, a.t, ax, ay)),
          ),
        );
    }
    if (worst > 3) offenders.push(`${name} ${worst.toFixed(2)}/frame`);
  }
  assert.deepEqual(offenders, [], `temporal steps: ${offenders.join(", ")}`);
});

/**
 * THE SEAM GATE — the defect the banding gate actually caught first.
 *
 * The three-stop ramp is an open path: position 0 is stop 0, position 1 is
 * stop 2, and they are the two most distant colours in the palette. A phase
 * axis is cyclic, so driving the ramp with a raw wrap puts a full-palette
 * discontinuity somewhere on screen at all times — measured at 182.5 sRGB on
 * buzz-dark before `cyclicRampPosition` folded the axis into a palindrome.
 *
 * Asserting only the interior buckets cannot see this: the seam lives exactly
 * at the wrap, which is the one adjacency an interior sweep skips. So this
 * checks the wrap-around pair explicitly, on every theme.
 */
test("the hue axis has no seam at the phase wrap", () => {
  // The fold itself: adjacent phases across the wrap map to adjacent ramp
  // positions, and the ends of the ramp are turning points, not neighbours.
  const epsilon = 1e-4;
  assert.ok(
    Math.abs(cyclicRampPosition(1 - epsilon) - cyclicRampPosition(0)) < 1e-3,
    "fold is discontinuous at the wrap",
  );
  assert.ok(
    Math.abs(cyclicRampPosition(0.5 - epsilon) - cyclicRampPosition(0.5)) <
      1e-3,
    "fold is discontinuous at the turn",
  );

  const offenders = [];
  for (const [name, p] of THEMES) {
    const table = buildBannerColorTable(p);
    // Walk BOTH waves' real hue axes across a full cycle, including the wrap, at
    // a fixed cell: consecutive samples must never jump. The field is the layer
    // the 182.5 seam was measured on, so leaving it out — as my first version of
    // this gate did — tests the wrong wave.
    let previousHead = null;
    let previousField = null;
    let max = 0;
    const steps = 1200;
    for (let step = 0; step <= steps; step += 1) {
      const phase = step / steps;
      const hue = makeCellHue(COLUMNS, ROWS, { field: phase, wordmark: phase });
      const head = table.lookup("head", 0, hue("head", 0.5, 0, 0));
      const field = table.lookup("field", 0.5, hue("field", 0.5, 70, 30));
      if (previousHead) max = Math.max(max, perceptualStep(previousHead, head));
      if (previousField)
        max = Math.max(max, perceptualStep(previousField, field));
      previousHead = head;
      previousField = field;
    }
    // Per-sample bound of 3 dE00 at 1200 samples per cycle, finer than 60Hz over
    // a 9.5s period, so a real seam cannot hide between samples. Generous against
    // sampling noise, brutal against a full-palette discontinuity: the unfolded
    // seam measured 182.5 sRGB / 55.6 dE00 on buzz-dark.
    if (max > 3) offenders.push(`${name} ${max.toFixed(2)}`);
  }
  assert.deepEqual(offenders, [], `temporal seams: ${offenders.join(", ")}`);
});

/**
 * THE INTERPOLATED WORDMARK FLOOR — the gate that caught a real WCAG failure.
 *
 * The contrast gate above walks table SAMPLES. That is not what paints: `lookup`
 * interpolates between samples, and this gate exists because the two disagree.
 * `bannerStops` solves each wordmark stop to sit EXACTLY on 4.5, and contrast is
 * convex along a straight sRGB blend, so the chord between two on-floor samples
 * passes UNDER the floor. Measured 4.4766 on solarized-light at hue 0.2188
 * before `sampleHead` re-lifted the blend.
 *
 * A sampled-only gate is blind to this by construction, and no bucket count
 * fixes it — the dip shrinks asymptotically and never reaches zero. So this
 * walks the axis at 4000 points per theme, far finer than any display samples it.
 */
test("interpolated wordmark colour never dips below the 4.5 WCAG floor", () => {
  const offenders = [];
  for (const [name, p] of THEMES) {
    const table = buildBannerColorTable(p);
    let worst = Number.POSITIVE_INFINITY;
    for (let step = 0; step <= 4000; step += 1) {
      worst = Math.min(
        worst,
        ratio(table.lookup("head", 0, step / 4000), p.background),
      );
    }
    // No epsilon on the floor itself: 4.5 is the standard, and the whole point
    // of this gate is that a 0.023 shortfall is still a shortfall. A tiny
    // tolerance for float noise only.
    if (worst < WORDMARK_FLOOR - 1e-9)
      offenders.push(`${name} ${worst.toFixed(4)}`);
  }
  assert.deepEqual(offenders, [], `below floor: ${offenders.join(", ")}`);
});

/**
 * THE INTENSITY CLAMP, asserted rather than merely documented.
 *
 * `BannerWaveConfig.intensity` documents its range as 0..1 and promises values
 * outside it are clamped. A mutant proved that promise was unverified: deleting
 * the `Math.max(0, Math.min(1, ...))` left every gate green, because all of them
 * drove intensity to values that were already in range. Documented-and-unchecked
 * is how a knob becomes a trap — exactly the failure mode Eva flagged.
 *
 * The clamp is what makes the safe-by-construction argument sound: intensity only
 * narrows which SUBRANGE of the hue axis is used, and the floors are proven over
 * the whole axis, so in-range values cannot reach an unproven colour. Without the
 * clamp, intensity 9 WIDENS the position past the axis and the argument collapses.
 * So this asserts the mechanism the argument depends on.
 */
test("intensity is clamped to 0..1 so it can never widen past the proven axis", () => {
  const columns = 112;
  const rows = 46;
  const inRange = (value) => value >= -1e-9 && value <= 1 + 1e-9;

  // Out-of-range settings must behave EXACTLY as their clamped equivalents, and
  // must never produce a hue position outside the axis the floors are proven on.
  const cases = [
    [9, 1],
    [-5, 0],
    [1e6, 1],
  ];
  for (const [wild, clamped] of cases) {
    const wildHue = makeCellHue(
      columns,
      rows,
      { field: 0.3, wordmark: 0.3 },
      {
        field: { ...FIELD_WAVE, intensity: wild },
        wordmark: { ...WORDMARK_WAVE, intensity: wild },
      },
    );
    const clampedHue = makeCellHue(
      columns,
      rows,
      { field: 0.3, wordmark: 0.3 },
      {
        field: { ...FIELD_WAVE, intensity: clamped },
        wordmark: { ...WORDMARK_WAVE, intensity: clamped },
      },
    );
    for (let row = 0; row < rows; row += 5)
      for (let column = 0; column < columns; column += 5) {
        const field = wildHue("field", 0.5, column, row);
        assert.ok(
          inRange(field),
          `intensity ${wild} put field hue at ${field}, outside the proven axis`,
        );
        assert.equal(
          field,
          clampedHue("field", 0.5, column, row),
          `intensity ${wild} did not behave as ${clamped}`,
        );
      }
    for (let step = 0; step <= 20; step += 1) {
      const head = wildHue("head", step / 20, 0, 0);
      assert.ok(
        inRange(head),
        `intensity ${wild} put wordmark hue at ${head}, outside the proven axis`,
      );
      assert.equal(head, clampedHue("head", step / 20, 0, 0));
    }
  }
});

/**
 * THE WORDMARK ACTUALLY ANIMATES, measured on PAINTED OUTPUT.
 *
 * A mutant found this gap, and it was a serious one: swapping the table's
 * wordmark lookup from the wave's hue to the cell's geometric `t`
 * (`sampleHead(hue)` -> `sampleHead(t)`) kills the wordmark wave stone dead — half
 * of what Tyler asked for — and every other gate stayed green.
 *
 * Why they all missed it: the direction and seam gates call `makeCellHue` and
 * `lookup` themselves, passing the hue in explicitly, so they verify the hue
 * FUNCTION and the TABLE in isolation. Nothing checked that the painter feeds one
 * into the other for the head layer. The field was covered only by luck — its
 * gates happen to go through the painter.
 *
 * So this asserts the composition, on the pixels: over a phase sweep, the
 * wordmark's painted colours must change, and within a single frame they must
 * vary along the mark's ink span. Both halves are needed — a wordmark frozen at
 * one colour satisfies neither, and one painted by `t` alone satisfies the second
 * but not the first.
 */
test("the wordmark's painted colours animate with phase", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const [name, p] = THEMES[0];
  const table = buildBannerColorTable(p);

  const headFrame = (phase) => {
    const target = recordingCanvas();
    paintTerminalBanner(target.canvas, banner, p, 1, {
      phase: { field: phase, wordmark: phase },
      table,
    });
    // Pick out the head layer's draws. `paintTerminalBanner` emits one draw per
    // non-blank layered cell in geometry order, so walking the geometry the same
    // way keeps the two in step.
    const heads = [];
    let index = 0;
    for (const cells of banner.cells)
      for (const cell of cells) {
        if (cell.char === " " || !cell.layer) continue;
        if (cell.layer === "head") heads.push(target.draws[index].color);
        index += 1;
      }
    return heads;
  };

  const frames = PHASE_SWEEP.map(headFrame);
  assert.ok(
    frames[0].length > 50,
    `${name} painted ${frames[0].length} head cells`,
  );

  // 1. The wordmark must not be a still: distinct frames across the sweep.
  const distinctFrames = new Set(frames.map((frame) => frame.join(""))).size;
  assert.ok(
    distinctFrames >= PHASE_SWEEP.length - 1,
    `wordmark produced only ${distinctFrames} distinct frames over ${PHASE_SWEEP.length} phases`,
  );

  // 2. And within a frame it must be a WAVE, not a flat wash.
  for (const [index, frame] of frames.entries()) {
    assert.ok(
      new Set(frame).size > 2,
      `frame ${index} painted the wordmark in only ${new Set(frame).size} colour(s)`,
    );
  }
});

/**
 * KNOB EXTREMES. Eva: "a knob that can silently break WCAG is a trap, not a
 * feature" — so the floors are gated at the documented ends of each knob's
 * range, not just at the defaults.
 *
 * The documented ranges, and why they are what they are:
 *  - intensity 0..1        SAFE THROUGHOUT, and by construction rather than by
 *                          luck: intensity only narrows which SUBRANGE of the
 *                          hue axis is used, and the floors are proven over the
 *                          ENTIRE axis. Out-of-range values are clamped.
 *  - wavelength .125..2.0  SAFE. Contrast is independent of wavelength (it only
 *                          rescales position along an axis whose floors are
 *                          proven end to end), so the binding constraint is
 *                          spatial busyness: MEASURED in dE00, worst excess over
 *                          a theme's own static step is 0.47 at the default and
 *                          under 0.62 through 2.0; at 4.0 it is 8.92 and fails
 *                          24 themes.
 *  - direction any vector  SAFE. Direction selects which bucket a cell reads and
 *                          cannot reach an unsampled colour.
 *  - seconds > 10          Not a contrast knob at all; gated separately as
 *                          "slow".
 */
test("contrast floors hold at every documented knob extreme", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const extremes = {
    "intensity=0": { intensity: 0 },
    "intensity=1": { intensity: 1 },
    "intensity=-5 (clamps to 0)": { intensity: -5 },
    "intensity=9 (clamps to 1)": { intensity: 9 },
    "wavelength=0.125": { wavelength: 0.125 },
    "wavelength=2.0": { wavelength: 2.0 },
    "direction=(1,1)": { direction: { x: 1, y: 1 } },
    "direction=(-1,0)": { direction: { x: -1, y: 0 } },
    "direction=(0,-1)": { direction: { x: 0, y: -1 } },
  };
  const offenders = [];
  // Six themes, not all 62: this is the same colour surface the all-theme gates
  // already cover, crossed against nine settings and a phase sweep. The extremes
  // question is whether a KNOB can reach an unproven colour, and the LUT is
  // shared across knobs, so theme breadth is covered elsewhere.
  for (const [name, p] of THEMES.slice(0, 6)) {
    const table = buildBannerColorTable(p);
    for (const [label, override] of Object.entries(extremes)) {
      const waves = {
        field: { ...FIELD_WAVE, ...override },
        wordmark: { ...WORDMARK_WAVE, ...override },
      };
      for (const phase of PHASE_SWEEP) {
        const target = recordingCanvas();
        paintTerminalBanner(target.canvas, banner, p, 1, {
          phase: { field: phase, wordmark: phase },
          table,
          waves,
        });
        assert.ok(target.draws.length > 1000, `${label} drew nothing`);
        for (const { color } of target.draws) {
          const cr = ratio(color, p.background);
          // Every painted colour must sit inside the union of the layers'
          // envelopes: the wordmark at or above 4.5, the field within its decay
          // band. Below the field's edge floor nothing is legible at all.
          if (cr < FIELD_EDGE - 0.02)
            offenders.push(`${name} ${label} phase ${phase}: ${cr.toFixed(3)}`);
        }
      }
    }
  }
  assert.deepEqual(offenders.slice(0, 6), [], `${offenders.length} violations`);
});

/**
 * The knob-extreme gate above is only as good as its ability to SEE a violation,
 * and it paints through the real painter, so it is worth proving it is not
 * vacuous. An intensity that pushed colours outside the proven axis would be
 * caught; here the axis is deliberately overrun to show the instrument fires.
 */
test("the knob-extreme gate can actually detect an out-of-envelope colour", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  // A table whose field lookup returns the background itself: contrast 1.0,
  // below the 1.04 edge floor. If the gate's predicate is wrong this passes.
  const broken = {
    ...table,
    lookup: (layer, t, hue) =>
      layer === "field" ? p.background : table.lookup(layer, t, hue),
  };
  const target = recordingCanvas();
  paintTerminalBanner(target.canvas, banner, p, 1, {
    phase: { field: 0.3, wordmark: 0.3 },
    table: broken,
  });
  const violations = target.draws.filter(
    ({ color }) => ratio(color, p.background) < FIELD_EDGE - 0.02,
  );
  assert.ok(
    violations.length > 100,
    `predicate saw only ${violations.length} violations on a deliberately broken table`,
  );
});

/**
 * The animated painter must colour cells by their WAVE position, not by their
 * geometric `t`. If the painter ignored the phase argument this would pass
 * trivially, so the assertion is that cells sharing a `t` but sitting in
 * different columns get DIFFERENT colours — which only a projection-derived hue
 * axis produces.
 */
test("animated field colour varies along a row, not just radially", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  const target = recordingCanvas();
  paintTerminalBanner(target.canvas, banner, p, 1, {
    phase: { field: 0.3, wordmark: 0.3 },
    table,
  });
  // Group field-layer draws by y, and check colour varies across x within a row.
  const rows = new Map();
  for (const { args, color } of target.draws) {
    const [, x, y] = args;
    if (!rows.has(y)) rows.set(y, []);
    rows.get(y).push({ x, color });
  }
  const varied = [...rows.values()].filter(
    (cells) => new Set(cells.map((cell) => cell.color)).size > 2,
  );
  assert.ok(
    varied.length > 5,
    `only ${varied.length} rows show horizontal colour variation`,
  );
});

/** Successive frames must actually differ, or the "animation" is a still. */
test("successive frames differ and a full cycle returns to the start", () => {
  const banner = buildTerminalBanner(COLUMNS, ROWS, ASPECT);
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  const frame = (seconds) => {
    const target = recordingCanvas();
    paintTerminalBanner(target.canvas, banner, p, 1, {
      phase: phaseAt(seconds),
      table,
    });
    return target.draws.map((draw) => draw.color).join("");
  };
  const a = frame(0);
  // One second of a 13s traversal is ~8 columns of drift: visible, not static.
  assert.notEqual(a, frame(1));
  // Seamless loop: the least common period of the two waves returns both to 0.
  // With 13s and 9.5s periods that is 247s (13*19 = 9.5*26), NOT the raw
  // product 123.5, which is 9.5 field cycles — half a cycle off.
  assert.equal(a, frame(2 * FIELD_PERIOD_SECONDS * WORDMARK_PERIOD_SECONDS));
});

/** The table is what makes this affordable, and its shape is load-bearing. */
test("the colour table is phase-independent and completely populated", () => {
  const [, p] = THEMES[0];
  const table = buildBannerColorTable(p);
  assert.equal(table.head.length, HUE_BUCKETS);
  assert.equal(table.field.length, CONTRAST_BUCKETS);
  for (const row of table.field) assert.equal(row.length, HUE_BUCKETS);
  for (const colour of [...table.head, ...table.field.flat()])
    assert.match(colour, /^#[0-9a-f]{6}$/);
});
