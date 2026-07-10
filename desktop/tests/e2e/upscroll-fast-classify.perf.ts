import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Fast-corpus reversal characterization — the SKIP admission gate.
 *
 * WHY THIS EXISTS. The W4a gate (`upscroll-raf-correction.perf.ts`) drives a
 * CONSTANT 12px/32ms (~375px/s) upscroll and, on WebKit, leaves 3-4 bounded
 * reversal survivors (rows mock-jitter-387/393/381/375) that PASS the ≤4 gate.
 * The classifier that typed those survivors as SKIP (fired=false,
 * signedShift=0.0 → momentum-skip gate) was run against a fast-drive variant of
 * the slow-scroll leg that was NEVER committed — so the SKIP labels were not
 * reproducible from the tree. This fixture is that fast-drive classifier,
 * committed, so the admission evidence is permanent and re-runnable. It reuses
 * the slow leg's `probeLen` append-count join verbatim and adds two Leg-5
 * cross-checks (thread event 2a4e31fa, Eva's admission-gate ruling):
 *
 *   1. `dev = rowMove − scrollDelta` per reversal — Leg 5's rendered deviation
 *      from pure scroll-tracking. On a SKIP frame scrollDelta≈0 so dev≈rowMove
 *      with NO fired write behind it = abandonment, not a corrector footprint.
 *   2. A WIDEN-INDEPENDENT neighborhood dump. Dawn's class attribution picks the
 *      single largest-|signedShift| attempt in a ±1-frame append-count window;
 *      the ±1 widen is the soft joint Eva flagged. This fixture ALSO reports,
 *      for every reversal, whether ANY `wouldFire=true` record exists in a
 *      WIDER ±2-frame window — attribution-free. If no fired write sits near a
 *      survivor at any reasonable window width, SKIP is robust to the widen; if
 *      one does, the largest-|shift| rule would have labelled it grow/shrink and
 *      the SKIP bin is in question. That neighborhood flag is the admission gate.
 *
 * HONESTY BOUND (unchanged from the slow leg): Playwright `mouse.wheel` is a
 * synthetic discrete event; the WebKit `dScroll=0.0` coalesced still frame is a
 * real-device phenomenon. But the fast gate corpus DOES surface the bounded
 * survivors on Playwright WebKit, so this fixture reproduces the frames the
 * admission gate must rule on. It CHARACTERIZES; it does not gate a ceiling.
 */

// Fast constant drive — identical to the W4a gate (`upscroll-raf-correction`),
// so this fixture surfaces the same bounded survivors the gate leaves.
const WHEEL_DELTA = 12; // px/event — matches the gate's constant velocity
const WHEEL_PERIOD_MS = 32; // gate cadence (~375px/s)
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

test("W4a fast-classify: SKIP admission — no fired write near the survivors", async ({
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

  // Fast constant drive — matches the W4a gate exactly, so the same bounded
  // survivors surface. No decay: this is the fast regime, not Tyler's slow one.
  const started = Date.now();
  while (Date.now() - started < DURATION_MS) {
    await page.mouse.wheel(0, -WHEEL_DELTA);
    await page.waitForTimeout(WHEEL_PERIOD_MS);
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
        renderedScroll?: number;
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
    dev: number; // Leg 5: rowMove − scrollDelta (rendered deviation from tracking)
    signedShift: number | null;
    fired: boolean;
    klass: Klass;
    // Widen-independent admission flag: any wouldFire=true record in a WIDER
    // ±2-frame append-count window than the ±1 attribution window. If false,
    // no fired write sits near this reversal at any reasonable width → SKIP is
    // robust to the widen. If true, the class attribution's largest-|shift| rule
    // could have labelled it grow/shrink and the SKIP bin is in question.
    firedNear: boolean;
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
    // Leg 5 rendered deviation from pure scroll-tracking. A correctly-anchored
    // row moves only with scroll (rowMove == scrollDelta), so any deviation is
    // the corrector's footprint — or, on a SKIP, its ABSENCE.
    const dev = rowMove - dScroll;
    // Widen-independent admission check. Look one frame WIDER than the ±1
    // attribution window ([i-2 .. i+2] via probeLen) and ask only: is there ANY
    // fired write in that neighborhood? This does not pick a single attempt or
    // depend on the largest-|shift| tie-break, so it cannot be flipped by the
    // widen. A SKIP survivor must have firedNear=false: no write could be the
    // backward mover if none fired near the frame at all.
    const lo = frames[i - 2] ?? a;
    const hi = frames[i + 2] ?? next;
    const neighborhood = corrections.slice(lo.probeLen, hi.probeLen);
    const firedNear = neighborhood.some((c) => c.wouldFire);
    reversals.push({
      i,
      rowMove,
      dScroll,
      dev,
      signedShift: attempt?.signedShift ?? null,
      fired: attempt?.wouldFire ?? false,
      klass,
      firedNear,
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
  console.log("\n=== W4a FAST-CORPUS SKIP ADMISSION ===");
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
      `  frame ${r.i} rowMove=${r.rowMove.toFixed(1)} dScroll=${r.dScroll.toFixed(1)} dev=${r.dev.toFixed(1)} signedShift=${s} fired=${r.fired} firedNear=${r.firedNear} class=${r.klass} row=${r.rowId}`,
    );
  }
  console.log("========================================\n");
  /* eslint-enable no-console */

  // Sanity: the actuation actually produced a scored upscroll.
  expect(scored).toBeGreaterThan(50);
  // Stale-`dist` guard — a characterization on a stale bundle misleads exactly
  // like a stale gate run. Assert the experiment's stamp ran.
  expect(buildStamp).toBe(EXPECTED_BUILD_STAMP);
  // Liveness: at least one mid-history correction fired, else the corpus
  // realized nothing and the distribution above is vacuous.
  const anyFired = corrections.some((c) => c.wouldFire);
  expect(anyFired).toBe(true);

  // --- SKIP ADMISSION GATE (Eva, thread event 2a4e31fa) -----------------------
  // Every reversal typed SKIP must have NO fired write in its ±2-frame
  // neighborhood. This is attribution-free: it does not depend on the ±1 widen
  // or the largest-|shift| tie-break, so a SKIP that survives it is robust to
  // the soft joint in the classifier. If any SKIP shows firedNear=true, a write
  // did land near the frame and the class attribution mis-labelled it — the bin
  // is not admissible and this fails loudly rather than passing a stale claim.
  // (Characterization otherwise; the reversal count itself is not gated.)
  const skips = reversals.filter((r) => r.klass === "skip");
  for (const r of skips) {
    expect(
      r.firedNear,
      `SKIP survivor frame ${r.i} (row ${r.rowId}) has a fired write in its ±2-frame neighborhood — attribution is not widen-robust, bin in question`,
    ).toBe(false);
  }

  // --- DECOMPOSITION SELF-TEST (Eva: arm's first run must SHOW it holds) -------
  // The w4a-gate-1 fix keys the momentum gate off the RENDERED scroll
  // (`renderedScroll = aboveShift − ΔtopOffset`) the hook emits per rAF attempt,
  // not the raw `Δscrolltop`. The load-bearing claim is that `renderedScroll`
  // ISOLATES the painted wheel component by subtracting the reflow's own push on
  // the anchor — so it stays small on a genuine reflow even when `signedShift`
  // (the reflow) is large. Prove that independence from the emit, NOT the gate's
  // own branch (asserting the gate skips when |renderedScroll|>bound is
  // circular). We compare two populations of rAF attempts:
  //   • PURE-REFLOW — a real above-anchor reflow (`|signedShift|` well past the
  //     0.5px epsilon) on a rendered-still frame. If the decomposition works,
  //     `renderedScroll` here is SMALL (the reflow push was removed), NOT tracking
  //     `signedShift`. This is the WebKit survivor the raw gate dropped.
  //   • PURE-SCROLL — no reflow (`signedShift` ≈ 0). `renderedScroll` here is
  //     free to be large: it is the wheel motion, with nothing to subtract.
  // The proof: pure-reflow's median |renderedScroll| is well BELOW its median
  // |signedShift| — the reflow did not leak into the gated quantity — while
  // pure-scroll shows |renderedScroll| CAN run large. If renderedScroll merely
  // echoed signedShift (a broken decomposition) the reflow bin would fail this.
  const median = (xs: number[]): number => {
    if (xs.length === 0) return 0;
    const s = [...xs].sort((p, q) => p - q);
    return s[Math.floor(s.length / 2)];
  };
  const rafWithRendered = corrections.filter(
    (c): c is typeof c & { renderedScroll: number } =>
      c.source === "raf" && typeof c.renderedScroll === "number",
  );
  const pureReflow = rafWithRendered.filter((c) => Math.abs(c.signedShift) > 5);
  const pureScroll = rafWithRendered.filter(
    (c) => Math.abs(c.signedShift) <= 0.5,
  );
  const reflowMedRendered = median(
    pureReflow.map((c) => Math.abs(c.renderedScroll)),
  );
  const reflowMedShift = median(pureReflow.map((c) => Math.abs(c.signedShift)));
  const scrollMaxRendered = pureScroll.length
    ? Math.max(...pureScroll.map((c) => Math.abs(c.renderedScroll)))
    : 0;
  /* eslint-disable no-console */
  console.log("=== DECOMPOSITION SELF-TEST (w4a-gate-1) ===");
  console.log(`rAF attempts w/ renderedScroll: ${rafWithRendered.length}`);
  console.log(`pure-reflow attempts (|shift|>5):   ${pureReflow.length}`);
  console.log(
    `  median |signedShift|:             ${reflowMedShift.toFixed(1)}`,
  );
  console.log(
    `  median |renderedScroll|:          ${reflowMedRendered.toFixed(1)}`,
  );
  console.log(
    `  fired:                            ${pureReflow.filter((c) => c.wouldFire).length}`,
  );
  console.log(`pure-scroll attempts (|shift|<=.5): ${pureScroll.length}`);
  console.log(
    `  max |renderedScroll|:             ${scrollMaxRendered.toFixed(1)}`,
  );
  console.log("============================================\n");
  /* eslint-enable no-console */
  // Both populations must be exercised, else the decomposition is untested.
  // Assertions are WEBKIT-ONLY: the rAF momentum gate is the ACTIVE corrector
  // only on WebKit (the RO is late). On Chromium the on-time RO corrects and
  // refreshes the cache first, so the rAF path is the passive loser observer —
  // it never fires (ratified mechanism) and reads the reflow BEFORE the RO's
  // compensation, so `renderedScroll` there does not cancel and tracks
  // `signedShift` instead. That is the correct Chromium behavior, not a
  // decomposition failure, so we characterize it (logged above) but only assert
  // the decomposition on the engine whose gate the fix rekeyed.
  if (browserName === "webkit") {
    expect(pureReflow.length).toBeGreaterThan(0);
    expect(pureScroll.length).toBeGreaterThan(0);
    // THE PROOF: on real reflow frames the reflow does NOT leak into
    // renderedScroll — its median stays well below the reflow magnitude (a
    // broken decomposition that echoed signedShift would fail this).
    expect(reflowMedRendered).toBeLessThan(reflowMedShift);
    // And a genuine rendered-still reflow is let through the gate — the survivor
    // the raw-delta gate dropped on WebKit's coalesced clock. If none fires the
    // rekey did nothing.
    expect(pureReflow.some((c) => c.wouldFire)).toBe(true);
  }
});
