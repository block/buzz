import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Slow-scroll characterization leg — Tyler's felt-regime coverage.
 *
 * The W4a gate (`upscroll-raf-correction.perf.ts`) drives a CONSTANT 12px/32ms
 * (~375px/s) upscroll. That is not the regime Tyler reports the residual in: a
 * slow deliberate trackpad gesture moves 2-4px/frame and decays through a
 * momentum tail, and a mid-history correction that is invisible under momentum
 * is 5-7 frames of motion at 2-3px/frame (Eva's characterization, thread
 * event 2de99f4d). This leg drives that regime and CHARACTERIZES the reversal
 * distribution — it does NOT gate a pass/fail ceiling, because the slow-regime
 * floor is exactly what we're measuring. Liveness + stale-`dist` guard only.
 *
 * HONESTY BOUND (flagged in-thread before build): Playwright `mouse.wheel` is a
 * synthetic discrete event. It faithfully reproduces "the correction WRITE as a
 * felt jump" (correction magnitude vs per-frame rendered row motion at low
 * velocity — measured here) but it does NOT reproduce a real trackpad's
 * compositor-coalesced momentum, so the WebKit `dScroll=0.0` coalesced still
 * frame is a real-device phenomenon this fixture cannot manufacture. A green
 * run here means "the correction is not a large felt jump in a low-velocity
 * synthetic gesture" — NOT "reproduces everything Tyler feels."
 */

// Momentum-decay drive: initial slow velocity decaying toward a tail. Each
// wheel event is small (2-4px early, ~1px in the tail) so a correction landing
// mid-gesture is many frames of the eye's motion, not one.
const WHEEL_START_DELTA = 4; // px/event at gesture start (slow deliberate)
const WHEEL_TAIL_DELTA = 1; // px/event in the momentum tail
const DECAY_PER_EVENT = 0.98; // geometric decay toward the tail
const WHEEL_PERIOD_MS = 16; // one event per frame (60fps trackpad cadence)
const DURATION_MS = 12_000;
const SAFE_MARGIN = 100;
// Same reversal definition as the gate: row moving against the scroll by more
// than staircase noise. Upscroll → rowMove normally >= 0, so a genuine
// against-direction move is < -REVERSAL_PX.
const REVERSAL_PX = 3;
// Must equal `ANCHOR_BUILD_STAMP` in `useAnchoredScroll.ts` — stale-`dist`
// guard (see the gate fixture). Bump BOTH together per experiment.
const EXPECTED_BUILD_STAMP = "w4a-gate-1";

type Frame = {
  t: number;
  top: number | null;
  scrollTop: number;
  mounted: number;
  rowId: string | null;
  probeLen: number;
};

test("W4a slow-scroll: reversal distribution in the felt low-velocity regime", async ({
  page,
  browserName,
}) => {
  await installMockBridge(page);
  page.on("console", (m) => {
    if (m.type() === "error") console.log("PAGE ERROR:", m.text());
  });
  page.on("pageerror", (e) => console.log("PAGE EXCEPTION:", e.message));
  await page.addInitScript(() => {
    (
      globalThis as unknown as { __ANCHOR_PROBE__: unknown[] }
    ).__ANCHOR_PROBE__ = [];
  });
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.getByTestId("channel-jitter-corpus").click();
  const timeline = page.getByTestId("message-timeline");
  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.scrollHeight > el.clientHeight + 1000;
  });

  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.style.overflowAnchor = "none";
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(200);
  await timeline.hover();

  // Per-frame sampler (identical row-tracking to the gate, plus a join key into
  // the hook's correction probe). The fixture sampler and the hook's rAF sampler
  // are SEPARATE rAF loops, so "the correction for frame i" is not reliably the
  // same-tick probe entry (two rAF callbacks in one frame fire in registration
  // order, which we don't control). The order-robust join is by APPEND COUNT:
  // each frame records `probeLen` (the correction-probe array length at that
  // tick) and the signed shift + fire flag of any attempts that appended since
  // the previous frame. A reversal between frame i-1 and i is then attributed to
  // the attempts in that interval — no same-tick ordering assumption.
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const w = window as unknown as {
      __PROBE__: { frames: Frame[]; stop: boolean };
    };
    const g = globalThis as unknown as {
      __ANCHOR_PROBE__?: Array<{
        wouldFire: boolean;
        residual: number;
        signedShift: number;
      }>;
    };
    type Frame = {
      t: number;
      top: number | null;
      scrollTop: number;
      mounted: number;
      rowId: string | null;
      // Correction-probe array length at this tick — the append-count join key.
      probeLen: number;
    };
    w.__PROBE__ = { frames: [], stop: false };
    let trackedId: string | null = null;
    const pick = (): string | null => {
      const box = el.getBoundingClientRect();
      for (const row of el.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const rect = row.getBoundingClientRect();
        if (rect.top > box.top + margin && rect.bottom < box.bottom - margin) {
          return row.dataset.messageId ?? null;
        }
      }
      return null;
    };
    const tick = (t: number) => {
      if (w.__PROBE__.stop) return;
      const mounted = el.querySelectorAll("[data-message-id]").length;
      let top: number | null = null;
      if (trackedId) {
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(trackedId)}"]`,
        );
        if (row) {
          const rect = row.getBoundingClientRect();
          const box = el.getBoundingClientRect();
          const inBand =
            rect.top > box.top + margin && rect.bottom < box.bottom - margin;
          top = inBand ? rect.top : null;
        }
      }
      if (top === null) trackedId = pick();
      w.__PROBE__.frames.push({
        t,
        top,
        scrollTop: el.scrollTop,
        mounted,
        rowId: trackedId,
        probeLen: g.__ANCHOR_PROBE__?.length ?? 0,
      });
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, SAFE_MARGIN);

  // Momentum-decay drive: velocity decays geometrically from START toward TAIL,
  // so the gesture spends most of its time in the 1-2px/frame regime where a
  // correction is felt. Fractional deltas accumulate a remainder so sub-pixel
  // velocity still produces integer wheel steps at the right average rate.
  const started = Date.now();
  let delta = WHEEL_START_DELTA;
  let remainder = 0;
  while (Date.now() - started < DURATION_MS) {
    remainder += delta;
    const step = Math.max(1, Math.round(remainder));
    remainder -= step;
    await page.mouse.wheel(0, -step);
    await page.waitForTimeout(WHEEL_PERIOD_MS);
    delta = Math.max(WHEEL_TAIL_DELTA, delta * DECAY_PER_EVENT);
  }

  const frames: Frame[] = await timeline.evaluate((_el) => {
    const w = window as unknown as {
      __PROBE__: { frames: Frame[]; stop: boolean };
    };
    type Frame = {
      t: number;
      top: number | null;
      scrollTop: number;
      mounted: number;
      rowId: string | null;
      probeLen: number;
    };
    w.__PROBE__.stop = true;
    return w.__PROBE__.frames;
  });

  const { corrections, buildStamp } = await page.evaluate(() => {
    const g = globalThis as unknown as {
      __ANCHOR_PROBE__?: Array<{
        source: "raf" | "ro";
        wouldFire: boolean;
        residual: number;
        signedShift: number;
      }>;
      __ANCHOR_BUILD_STAMP__?: string;
    };
    return {
      corrections: g.__ANCHOR_PROBE__ ?? [],
      buildStamp: g.__ANCHOR_BUILD_STAMP__ ?? null,
    };
  });

  // Score same-row frame pairs. A reversal is rowMove <= -REVERSAL_PX. For each
  // reversal, join to the correction attempt(s) that appended to the hook probe
  // BETWEEN frame i-1 and i (append-count window: probe indices [a.probeLen,
  // b.probeLen)). Classify by the SIGN of aboveShift + whether the write fired —
  // the three-way discriminator Sami specced (thread event 5b46582e):
  //   • wouldFire == false                → SKIP: momentum gate (:29) / cross-
  //     check (:451) suppressed the write. The reversal is UNCORRECTED reflow;
  //     absorption never got to act. A 27→27 here = wiring/gate, not physics.
  //   • fired, signedShift > 0 (GROW)     → content above grew, anchor shoved
  //     DOWN, the correction WRITE is the felt backward snap. Absorption's
  //     amortizable topology — deferring the pullback into forward frames helps.
  //   • fired, signedShift < 0 (SHRINK)   → content above shrank, the reflow
  //     ITSELF pulls the anchor up and renders the reversal before any write.
  //     Structurally uncorrectable by us; only smaller per-frame realization
  //     (Max's pre-realization band / contain-intrinsic-size) shrinks it.
  // A reversal with no attempt in its window is UNATTRIBUTED (the correcting
  // observer's attempt landed in an adjacent frame under rAF interleave) — we
  // count it separately rather than force it into a class.
  let scored = 0;
  let reanchors = 0;
  type Klass = "skip" | "grow" | "shrink" | "unattributed";
  const reversals: Array<{
    i: number;
    rowMove: number;
    dScroll: number;
    signedShift: number | null;
    fired: boolean;
    klass: Klass;
    rowId: string | null;
  }> = [];
  for (let i = 1; i < frames.length; i += 1) {
    const a = frames[i - 1];
    const b = frames[i];
    if (
      a.top === null ||
      b.top === null ||
      a.rowId === null ||
      a.rowId !== b.rowId
    ) {
      reanchors += 1;
      continue;
    }
    scored += 1;
    const rowMove = b.top - a.top;
    const dScroll = b.scrollTop - a.scrollTop;
    if (rowMove > -REVERSAL_PX) continue;
    // Attribution window. The hook's correction attempt for the reflow that
    // produced this reversal can append across a ±1-frame span relative to our
    // sampler: the two rAF loops interleave in an order we don't control, and on
    // WebKit a late RO appends a frame after the reflow paints. So the window is
    // [prev-frame probeLen, NEXT-frame probeLen) — attempts from the frame
    // before through the frame after. A reversal with NO attempt anywhere in
    // that span is genuinely unattributed (the corrector did not run a mid-
    // history attempt on those frames at all — e.g. re-pick guard or null cur),
    // which is itself a distinct diagnosis from a fired-then-clamped write.
    const next = frames[i + 1] ?? b;
    const window = corrections.slice(a.probeLen, next.probeLen);
    let attempt: (typeof corrections)[number] | null = null;
    for (const c of window) {
      if (
        attempt === null ||
        Math.abs(c.signedShift) > Math.abs(attempt.signedShift)
      ) {
        attempt = c;
      }
    }
    let klass: Klass;
    if (attempt === null) {
      klass = "unattributed";
    } else if (!attempt.wouldFire) {
      klass = "skip";
    } else {
      klass = attempt.signedShift >= 0 ? "grow" : "shrink";
    }
    reversals.push({
      i,
      rowMove,
      dScroll,
      signedShift: attempt?.signedShift ?? null,
      fired: attempt?.wouldFire ?? false,
      klass,
      rowId: b.rowId,
    });
  }

  const maxReversalPx =
    reversals.length === 0
      ? 0
      : Math.max(...reversals.map((r) => Math.abs(r.rowMove)));
  // A reversal on a near-still frame (|dScroll| < REVERSAL_PX) is the felt case:
  // the eye is barely moving, so a backward row snap is maximally visible.
  const stillFrameReversals = reversals.filter(
    (r) => Math.abs(r.dScroll) < REVERSAL_PX,
  );
  const byClass = (k: Klass) => reversals.filter((r) => r.klass === k).length;

  /* eslint-disable no-console */
  console.log("\n=== W4a SLOW-SCROLL CHARACTERIZATION ===");
  console.log(`engine:                 ${browserName}`);
  console.log(`build stamp:            ${buildStamp ?? "(absent)"}`);
  console.log(`frames sampled:         ${frames.length}`);
  console.log(`frame-pairs scored:     ${scored}`);
  console.log(`re-anchor/skip frames:  ${reanchors}`);
  console.log(`reversal frames:        ${reversals.length}`);
  console.log(`  of which still-frame: ${stillFrameReversals.length}`);
  console.log(`max reversal px:        ${maxReversalPx.toFixed(1)}`);
  console.log("--- reversal class mix (Sami's discriminator) ---");
  console.log(`  SKIP (gate/xcheck, uncorrected reflow):   ${byClass("skip")}`);
  console.log(
    `  GROW (fired, write is the snap — absorb):  ${byClass("grow")}`,
  );
  console.log(
    `  SHRINK (fired, reflow renders it — Max):   ${byClass("shrink")}`,
  );
  console.log(
    `  UNATTRIBUTED (no attempt in window):       ${byClass("unattributed")}`,
  );
  for (const r of reversals
    .slice()
    .sort((x, y) => x.rowMove - y.rowMove)
    .slice(0, 12)) {
    const s = r.signedShift === null ? "n/a" : r.signedShift.toFixed(1);
    console.log(
      `  frame ${r.i} rowMove=${r.rowMove.toFixed(1)} dScroll=${r.dScroll.toFixed(1)} signedShift=${s} fired=${r.fired} class=${r.klass} row=${r.rowId}`,
    );
  }
  console.log("========================================\n");
  /* eslint-enable no-console */

  // Sanity: the actuation actually produced a scored slow upscroll.
  expect(scored).toBeGreaterThan(50);
  // Stale-`dist` guard — a slow-regime characterization on a stale bundle would
  // mislead exactly like a stale gate run. Assert the experiment's stamp ran.
  expect(buildStamp).toBe(EXPECTED_BUILD_STAMP);
  // Liveness: at least one mid-history correction fired, else the corpus
  // realized nothing and the distribution above is vacuous.
  const anyFired = corrections.some((c) => c.wouldFire);
  expect(anyFired).toBe(true);
  // Characterization only — no reversal ceiling asserted. The distribution and
  // still-frame count above are the deliverable; the slow-regime floor is what
  // we are measuring, not gating.
});
