import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Media-corpus reversal characterization — classify MEDIA reflow snaps.
 *
 * WHY THIS EXISTS. The jitter corpus mis-estimates TEXT row heights; this one
 * mis-reserves MEDIA. The question this fixture answers (Eva's ruling, thread
 * event 0a496379): are the −240px media reflow snaps Tyler may feel on scroll-up
 * momentum-class (already killed by the w4a-gate-1 rekey → SKIP), walk-blind /
 * UNATTRIBUTED (a separate RO-path leg), or SHRINK-class (needs Max's pre-
 * realization band)? It reuses the fast classifier's machinery VERBATIM — the
 * `probeLen` append-count join, the SKIP/GROW/SHRINK/UNATTRIBUTED discriminator,
 * `dev`, and the ±2-frame `firedNear` admission flag — and only swaps the corpus,
 * narrows the viewport so the width-clamp SHRINK source is active, and relaxes
 * the SKIP-only admission assertion (media survivors are not all SKIP).
 *
 * THE CORPUS (channel `media-corpus`, e2eBridge `mediaCorpusBody`). Row height
 * error is MEDIA reserve-vs-true mismatch, not text mis-estimation, from three
 * in-source-verified divergence sources (Eva's premise verification, mainbase
 * 2cc0eb53):
 *   A. Link-preview cards — reserved flat at PREVIEW_CARD=70, rendered taller → GROW.
 *   B. Width-clamp images — CORRECT dims, but the estimator hardcodes
 *      MEDIA_MAX_WIDTH=384 while render clamps to `max-w-[min(24rem,100%)]`; in a
 *      column NARROWER than 384px the render is width-limited shorter → SHRINK.
 *      This fixture MEASURES the container width and asserts it is < 384 so the
 *      clamp provably bites (else the SHRINK source is silently inert).
 *   C. Dim-mismatch images, BOTH directions — overstate height → SHRINK, understate
 *      → GROW; seeded both ways so the corpus CAN produce SHRINK, making the
 *      GROW-dominant prediction falsifiable.
 * No dim-less band: that path is pinned to a fixed 256px box (reserve==mount) and
 * provably cannot reflow.
 *
 * PREDICTIONS ON RECORD (classifier decides): Dawn GROW-likely, Eva SHRINK-likely.
 *
 * HONESTY BOUND: this is a deterministic proxy for media reflow — a committed,
 * re-runnable classification of reserve-vs-true mismatch — NOT a claim to
 * reproduce Tyler's exact frames. His live trackpad still owns acceptance. The
 * WebKit `dScroll=0.0` coalesced still frame is the real-device phenomenon; the
 * synthetic `mouse.wheel` drive surfaces the bounded survivors to rule on.
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
// Narrow window so the timeline column lands below MEDIA_MAX_WIDTH=384 and the
// width-clamp SHRINK source (divergence B) is active. Measured in-test below.
const NARROW_VIEWPORT = { width: 450, height: 720 };
const MEDIA_MAX_WIDTH = 384; // must match rowHeightEstimate.ts

type Frame = {
  t: number;
  top: number | null;
  scrollTop: number;
  mounted: number;
  rowId: string | null;
  probeLen: number;
};

test("W4a media-classify: MEDIA reflow class mix (SKIP/GROW/SHRINK)", async ({
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
  // Navigate at the default (wide) viewport so the sidebar renders and the
  // channel is clickable, THEN narrow — a narrow window collapses the sidebar.
  await page.getByTestId("channel-media-corpus").click();
  await page.setViewportSize(NARROW_VIEWPORT);
  const timeline = page.getByTestId("message-timeline");
  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.scrollHeight > el.clientHeight + 1000;
  });

  // The width-clamp SHRINK source is only active if the rendered image
  // container is narrower than the estimator's hardcoded MEDIA_MAX_WIDTH=384.
  // Measure a mounted image block and assert it: if the column is wide the
  // clamp is inert and any SHRINK-absence below would prove nothing.
  const mediaBoxWidth = await page.evaluate(() => {
    const box = document.querySelector<HTMLElement>(
      '[data-testid="message-timeline"] [data-block-media]',
    );
    return box ? Math.round(box.getBoundingClientRect().width) : null;
  });
  expect(
    mediaBoxWidth,
    "no media block mounted in the timeline — corpus did not realize images",
  ).not.toBeNull();
  expect(
    mediaBoxWidth as number,
    `media container width ${mediaBoxWidth}px is not below MEDIA_MAX_WIDTH=${MEDIA_MAX_WIDTH} — width-clamp SHRINK source is inert; narrow the viewport further`,
  ).toBeLessThan(MEDIA_MAX_WIDTH);

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

  // Media-reflow census. The felt question is not "did any correction fire"
  // (text rows fire too) but "did an above-anchor MEDIA reflow, on a row the eye
  // is tracking, render a backward snap under the gate fix?" So for every frame
  // interval carrying a real reflow attempt (|signedShift| > REFLOW_PX) we record
  // whether the tracked row was in-band across it and what it actually did
  // (rowMove). An in-band reflow with rowMove ≈ 0 or forward is an ABSORBED media
  // reflow — the corpus produced the divergence and the gate+correction ate it.
  // Consecutive frames carrying the SAME (row, sign, fired) reflow are collapsed
  // to one census entry: a large above-anchor gap the momentum gate holds is
  // re-measured every rAF, and counting each frame would inflate one persistent
  // reflow into hundreds. We keep the WORST (most-backward) rowMove of the run so
  // a hidden snap inside a persistent reflow still surfaces.
  const REFLOW_PX = 3;
  type ReflowFrame = {
    i: number;
    shift: number;
    fired: boolean;
    inBand: boolean;
    worstRowMove: number | null;
    frames: number;
    rowId: string | null;
  };
  const mediaReflows: ReflowFrame[] = [];
  for (let i = 1; i < frames.length; i += 1) {
    const a = frames[i - 1];
    const b = frames[i];
    const attempts = corrections.slice(a.probeLen, b.probeLen);
    let biggest: (typeof attempts)[number] | null = null;
    for (const c of attempts) {
      if (Math.abs(c.signedShift) <= REFLOW_PX) continue;
      if (
        biggest === null ||
        Math.abs(c.signedShift) > Math.abs(biggest.signedShift)
      ) {
        biggest = c;
      }
    }
    if (biggest === null) continue;
    const inBand =
      a.top !== null &&
      b.top !== null &&
      a.rowId !== null &&
      a.rowId === b.rowId;
    const rowMove = inBand ? (b.top as number) - (a.top as number) : null;
    const sign = Math.sign(biggest.signedShift);
    const prev = mediaReflows[mediaReflows.length - 1];
    // Collapse a run: same tracked row, same reflow sign, same fire decision.
    if (
      prev &&
      prev.rowId === b.rowId &&
      Math.sign(prev.shift) === sign &&
      prev.fired === biggest.wouldFire &&
      prev.inBand === inBand
    ) {
      prev.frames += 1;
      if (Math.abs(biggest.signedShift) > Math.abs(prev.shift)) {
        prev.shift = biggest.signedShift;
      }
      if (
        rowMove !== null &&
        (prev.worstRowMove === null || rowMove < prev.worstRowMove)
      ) {
        prev.worstRowMove = rowMove;
      }
      continue;
    }
    mediaReflows.push({
      i,
      shift: biggest.signedShift,
      fired: biggest.wouldFire,
      inBand,
      worstRowMove: rowMove,
      frames: 1,
      rowId: b.rowId,
    });
  }
  const inBandReflows = mediaReflows.filter((r) => r.inBand);

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
  console.log("\n=== W4a MEDIA-CORPUS REFLOW CLASS MIX ===");
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

  console.log("=== MEDIA-REFLOW CENSUS (|shift|>3px above-anchor reflows) ===");
  console.log(`distinct reflow runs:    ${mediaReflows.length}`);
  console.log(`  in-band (tracked row): ${inBandReflows.length}`);
  console.log(
    `  GROW / SHRINK runs:    ${mediaReflows.filter((r) => r.shift > 0).length} / ${mediaReflows.filter((r) => r.shift < 0).length}`,
  );
  for (const r of mediaReflows) {
    console.log(
      `  frame ${r.i} shift=${r.shift.toFixed(1)} fired=${r.fired} inBand=${r.inBand} worstRowMove=${r.worstRowMove === null ? "n/a" : r.worstRowMove.toFixed(1)} frames=${r.frames} row=${r.rowId}`,
    );
  }
  console.log(
    "==============================================================\n",
  );
  /* eslint-enable no-console */

  // Sanity: the actuation actually produced a scored upscroll.
  expect(scored).toBeGreaterThan(50);
  // Stale-`dist` guard — a characterization on a stale bundle misleads exactly
  // like a stale gate run. Assert the experiment's stamp ran.
  expect(buildStamp).toBe(EXPECTED_BUILD_STAMP);
  // MEDIA LIVENESS. Not "did any correction fire" (text rows fire too) but "did
  // the MEDIA corpus actually produce above-anchor reflows on tracked rows?" If
  // it did not, the zero-reversal result below is vacuous (a dead corpus proves
  // nothing). This asserts the corpus is a LIVE reflow source — the divergence
  // bands realize — so the absence of felt reversals is a real absorption
  // result, not a silent no-op.
  expect(
    inBandReflows.length,
    "no |shift|>3px media reflow landed on a tracked in-band row — corpus did not realize media divergence, the reversal census is vacuous",
  ).toBeGreaterThan(0);

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
});
