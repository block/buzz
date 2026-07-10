import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * TRACKPAD-MOMENTUM UPSCROLL GATE (W3/W4) — the merge-blocking, offline sibling
 * of T1.2's sync gate. T1.2 guards the writer's MATH (fixed synchronous step,
 * per-notch median-of-run); this gate guards the INPUT PATH Tyler actually
 * feels (a real wheel under momentum, whose per-notch Blink scaling of
 * 218/220/222 poisons median-of-run). The two JOIN — sync + felt — neither
 * replaces the other.
 *
 * WHY FELT, NOT SYNC-ONLY (Quinn's sharpening, Eva-ratified). The sync path
 * UNDER-REPORTS the WebKit defect ~4-5x (sync reds ~41px, felt reds ~200px):
 * the wheel path exercises realization-under-momentum that the settled sync
 * path never does. Sync catching *a* red is not sync catching the *whole* red —
 * a sync-only gate can false-green a "fix" that still lurches 200px on the
 * trackpad. Gate the path the user's input takes.
 *
 * THE METRIC — two legs, both on the reading row's per-frame `rect.top`, NEITHER
 * referencing scrollTop nor the dispatched wheel delta. Full derivation +
 * invariance proofs: RESEARCH/FELT_WHEEL_GATE_METRIC_W4.md (Quinn, W4).
 *
 *   p_i       = tracked row's viewport top, sampled PER FRAME i
 *   rowMove_i = p_i − p_{i-1}                                   (first diff)
 *
 * LEG 1 — peak lurch = peak |second difference| (jerk):
 *   lurch_i = |p_i − 2·p_{i-1} + p_{i-2}| = |rowMove_i − rowMove_{i-1}|
 *   gate    = max_i lurch_i ≤ MAX_PEAK_JERK_PX
 *
 * WHY JERK SURVIVES WHEEL-SCALING WITHOUT A MEDIAN OR INPUT CLOCK. Blink's
 * scaling makes rowMove a SMOOTH ramp (218,220,222…); the second difference of
 * any locally-linear sequence is ≈ 0 ((222−220)−(220−218)=0), so scaling
 * flattens itself with NO reference. It is the coordinate-free version of Eva's
 * "subtract the per-notch expected value" — differencing twice removes any
 * smooth trend without naming it, so no dispatched-delta reference (Quinn's
 * "Trap A" / timing-skew). Comp-invariant: a working writer produces smooth row
 * motion (holding at ~0 OR tracking smoothly, both second-diff 0); a failing
 * writer produces a one-frame realization spike → second-diff spikes.
 *
 * WHY PER-FRAME IS CORRECT HERE (and why it does NOT repeat the mistake that
 * killed my median-of-run). median-of-run needed per-NOTCH settled sampling: a
 * zero-motion per-RAF frame read as a false stall against the nonzero run
 * median. Jerk is IMMUNE to that exact ghost — a paused frame is rowMove_i=0 AND
 * rowMove_{i-1}=0 → second-diff 0. The sampling grain follows the metric: jerk
 * is a frame-to-frame second difference and REQUIRES per-frame p_i (Quinn, W4).
 *
 * LEG 2 — skip-forever = signed cumulative drift over a trailing horizon:
 *   drift_H = Σ_{i in H} max(0, −sign(scrollDir)·rowMove_i)     (anti-scroll)
 * Jerk MISSES a *sustained* reversal (constant reversal → second-diff 0 too).
 * Signed drift catches it: scaling changes forward MAGNITUDE, never SIGN, so
 * scaled notches contribute 0 to the anti-scroll accumulator while a sustained
 * reversal accumulates. sign(scrollDir) is the coarse run direction, a constant
 * — not a per-frame input delta, so Leg 2 also touches no input clock.
 *
 * EXCLUSION SET (doc §Contract summary). Prepend-commit frames (page legitly
 * gains height) and writer skip-catchup frames are EXCLUDED from Leg 1 peak — a
 * one-frame reanchor is a real single event, not a lurch. Because jerk is a
 * second difference, excluding a frame means BREAKING the diff sequence at it:
 * we reset the diff run and only score jerk within contiguous CLEAN spans (span
 * (a) in the thread — cleaner than dropping straddling windows, and a real lurch
 * that coincides with a commit is not lost, it re-appears on the next clean
 * span's first scored frame). Excluded frames are KEPT in Leg 2 drift.
 *
 * ENGINE FIDELITY — same mirror as T1.1/T1.2: shipped WKWebView has no
 * `overflow-anchor`, so we force `overflow-anchor: none` and Chromium reproduces
 * the shipped engine. Under `perf-webkit` this is the real WebKit family.
 *
 * ASYMMETRIC TWO-ENGINE CONTRACT (Eva-ratified; matches T1.2). The engines DO
 * NOT share the defect (Dawn's white-box RO logs: WebKit's correction never
 * fires, Chromium's fires and holds). So:
 *   - WebKit @ contract (`6b9203ca`): MUST be RED — the load-bearing validity
 *     proof. Dawn's fix turns it green.
 *   - Chromium: GREEN @ contract AND @ fix — a no-regression guard on the engine
 *     that already works. There is NO Chromium contract defect; a red there
 *     could only be a metric artifact. DO NOT "fix" Chromium's green red.
 * If a future change greens WebKit at contract WITHOUT the fix, this gate is
 * VOID — do not relax thresholds; restore the mirror/corpus so WebKit reds.
 *
 * GUARDRAIL / FALSIFIER (doc §Acceptance): a correct writer (Chromium
 * T1.2-green) MUST score ~0 on BOTH legs under wheel actuation. If Leg 1 can't
 * hold Chromium ~0 under scaling, the second-diff invariance claim is FALSE and
 * we fall back to sync-only (option 2). The ceilings below are pinned only after
 * the Chromium-green baseline number is on the record.
 *
 * Run (Chromium):
 *   pnpm build && npx playwright test --config=playwright.perf.config.ts \
 *     upscroll-trackpad-gate
 * Run (WebKit — the engine that reds at tip):
 *   npx playwright test --config=playwright.perf-webkit.config.ts \
 *     upscroll-trackpad-gate --project=perf-webkit
 */

// LEG 1 ceiling: peak |second difference of rect.top| across per-frame samples
// within a clean span (px). Sits in the gap between the Chromium quantization
// floor (~2-3px at STEP_PX=2) and the WebKit realization spike (~27px). RED at
// tip: WebKit felt lurches spike peak jerk to ~27px (peak ≫ rms — a clean
// outlier). Chromium stays at the floor. GUARDRAIL: pinned only after the
// Chromium-green baseline (~2px) is on the record (WORK_LOG 2026-07-08).
const MAX_PEAK_JERK_PX = 8.0;
// LEG 2 ceiling: signed anti-scroll cumulative drift over a trailing horizon
// (px). Chromium scores 0.00 (huge headroom); WebKit ~25px. Sits between.
const MAX_DRIFT_PX = 12.0;
// LEG 3 ceiling: rms of |second diff| (jerk) over the same non-commit clean
// spans as Leg 1 — the CHATTER catcher (sign-balanced sub-peak lurches that
// dodge Leg 1's peak AND Leg 2's sign; RESEARCH/FELT_WHEEL_GATE_METRIC_W4.md
// §Leg 3). Under discrete integer actuation rms-jerk has an irreducible
// staircase floor ≈ f(STEP) (STEP=2 → ~2px), so this GATES only once a chattery
// R_bad is measured to clear the floor with margin (≥~4-5px). Until that A/B
// separation is on the record the leg is LOG-ONLY: set BUZZ_PERF_GATE_RMS_JERK=1
// to assert it, pinning the ceiling here between the floor and the R_bad number.
const MAX_RMS_JERK_PX = Number(process.env.BUZZ_PERF_MAX_RMS_JERK_PX ?? 4.0);
const GATE_RMS_JERK = process.env.BUZZ_PERF_GATE_RMS_JERK === "1";
// Trailing horizon for Leg 2 drift (ms). Long enough to contain a real
// sustained reversal, short enough not to dilute one; sits above a phase pair.
const DRIFT_HORIZON_MS = Number(process.env.BUZZ_PERF_DRIFT_HORIZON_MS ?? 100);

// Keep the tracked row this far (px) from both viewport edges so it stays
// realized across a frame — no straddling-row un-realization artifact.
const SAFE_MARGIN = 60;
// Swipes per run — enough to cross several fetchOlder pages on the 400-row seed.
const SWIPES = Number(process.env.BUZZ_PERF_SWIPES ?? 30);
// Deferred-commit latency for the mock fetchOlder page, so the prepend commits
// under continuous momentum (mirrors the live ~1s network fetch).
const FETCH_DELAY_MS = Number(process.env.BUZZ_PERF_FETCH_DELAY_MS ?? 1000);
// Constant per-event wheel delta (px). Continuous, no settle between events —
// realization happens WHILE scroll is in flight and more input keeps arriving
// (the momentum property). SMALL by design: Blink/WebKit deliver DISCRETE
// integer-px wheel events and rect.top is integer-quantized, so per-frame
// rowMove is a 0/STEP staircase whose second difference is STEP everywhere — a
// quantization FLOOR that scales with STEP. The WebKit realization shove is a
// FIXED physical displacement, independent of step size, so a SMALL step pulls
// the Chromium floor down (STEP=12 → 12px floor; STEP=2 → 2px floor) while the
// WebKit red holds (~27px) — signal-to-floor 4.6× → 13× (WORK_LOG 2026-07-08).
// This is what makes the raw per-frame 2nd-diff scorer survive without
// windowing: peak≫rms on WebKit is a clean outlier above a ~2px Chromium floor.
const STEP_PX = Number(process.env.BUZZ_PERF_STEP_PX ?? 2);

type Frame = {
  t: number;
  scrollTop: number;
  rowId: string | null;
  rowTop: number | null;
  mounted: number;
  fetch: number;
};

test("GATE: trackpad-momentum upscroll — peak jerk in rect.top stays below the felt-lurch threshold", async ({
  page,
}) => {
  test.setTimeout(900_000);
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.getByTestId("channel-jitter-corpus").click();
  await expect(page.getByTestId("chat-title")).toHaveText("jitter-corpus");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.scrollHeight > el.clientHeight + 1000;
  });

  // Defer the next fetchOlder page so it commits under momentum, not instantly.
  await page.evaluate((delayMs: number) => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, channelWindowDelayMs: delayMs },
    };
  }, FETCH_DELAY_MS);

  const anchorSupport = await page.evaluate(() =>
    typeof CSS !== "undefined" && typeof CSS.supports === "function"
      ? CSS.supports("overflow-anchor", "auto")
      : false,
  );

  // Pin to the true bottom so everything above is unpainted (at estimate), then
  // force the WKWebView mirror (no native scroll anchoring).
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
    (el as HTMLElement).style.overflowAnchor = "none";
  });
  await page.waitForTimeout(500);

  // ---- Per-RAF sampler, in page, independent of input cadence. Records the
  // tracked centre row's rect.top (viewport-relative) every frame. This is the
  // per-frame grain jerk requires; scrollTop is recorded ONLY as an activity
  // gate for the analysis, never fed into the scorer.
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const store = window as unknown as {
      __FRAMES__: Frame[];
      __SAMPLER_STOP__?: boolean;
      __CHANNEL_WINDOW_FETCH_COUNT__?: number;
    };
    type Frame = {
      t: number;
      scrollTop: number;
      rowId: string | null;
      rowTop: number | null;
      mounted: number;
      fetch: number;
    };
    store.__FRAMES__ = [];
    let trackedId: string | null = null;
    const pick = (): string | null => {
      const box = el.getBoundingClientRect();
      const mid = box.top + box.height / 2;
      let best: { id: string; d: number } | null = null;
      for (const row of el.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const r = row.getBoundingClientRect();
        if (r.top <= box.top + margin || r.bottom >= box.bottom - margin)
          continue;
        const d = Math.abs((r.top + r.bottom) / 2 - mid);
        if (!best || d < best.d) best = { id: row.dataset.messageId ?? "", d };
      }
      return best?.id || null;
    };
    const loop = () => {
      if (store.__SAMPLER_STOP__) return;
      const box = el.getBoundingClientRect();
      // Report the row's top RELATIVE to the container so a container reflow
      // does not read as row motion.
      let rowTop: number | null = null;
      if (trackedId) {
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(trackedId)}"]`,
        );
        if (row) {
          const r = row.getBoundingClientRect();
          if (r.top > box.top + margin && r.bottom < box.bottom - margin)
            rowTop = r.top - box.top;
        }
      }
      if (rowTop === null) {
        trackedId = pick();
        if (trackedId) {
          const r = el
            .querySelector<HTMLElement>(
              `[data-message-id="${CSS.escape(trackedId)}"]`,
            )
            ?.getBoundingClientRect();
          rowTop = r ? r.top - box.top : null;
        }
      }
      store.__FRAMES__.push({
        t: performance.now(),
        scrollTop: el.scrollTop,
        rowId: trackedId,
        rowTop,
        mounted: el.querySelectorAll("[data-message-id]").length,
        fetch: store.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0,
      });
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }, SAFE_MARGIN);

  // ---- Continuous constant-delta wheel input. No settle between events: input
  // keeps arriving through realization and prepend commits (the momentum
  // property). Blink scales the delta per notch; jerk reads through it.
  const box = await timeline.boundingBox();
  if (!box) throw new Error("no timeline box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  const isChromium =
    page.context().browser()?.browserType().name() === "chromium";
  const cdp = isChromium ? await page.context().newCDPSession(page) : null;
  await page.mouse.move(cx, cy);

  // Each swipe covers a fixed SCROLL DISTANCE, not a fixed event count, so a
  // smaller STEP_PX just means more (finer) events per swipe — the swipe still
  // crosses the same amount of history and pages a fetchOlder prepend. At the
  // live profile's ~816px/swipe (68 × 12px), STEP=2 → ~408 events/swipe.
  const SWIPE_DISTANCE_PX = 816;
  const EVENTS_PER_SWIPE = Math.max(1, Math.round(SWIPE_DISTANCE_PX / STEP_PX));
  for (let s = 0; s < SWIPES; s++) {
    for (let e = 0; e < EVENTS_PER_SWIPE; e++) {
      if (cdp) {
        await cdp.send("Input.dispatchMouseEvent", {
          type: "mouseWheel",
          x: cx,
          y: cy,
          deltaX: 0,
          deltaY: -STEP_PX,
          pointerType: "mouse",
        });
      } else {
        await page.mouse.wheel(0, -STEP_PX);
      }
      await new Promise((r) => setTimeout(r, 8));
    }
    await page.waitForTimeout(120);
    const at = await timeline.evaluate(
      (el) => (el as HTMLDivElement).scrollTop,
    );
    if (at <= 0) {
      // At the wall — let the deferred prepend land, then keep swiping.
      await page.waitForTimeout(FETCH_DELAY_MS + 1500);
    }
  }

  await page.evaluate(() => {
    (window as unknown as { __SAMPLER_STOP__?: boolean }).__SAMPLER_STOP__ =
      true;
  });
  const frames = (await page.evaluate(
    () => (window as unknown as { __FRAMES__: Frame[] }).__FRAMES__,
  )) as Frame[];

  // ---- Analysis: two legs on per-frame rect.top derivatives (Quinn's W4).
  //
  // Build the first-difference series rowMove_i = p_i − p_{i-1}, flagging frames
  // where the diff sequence must BREAK (an excluded frame): a re-pick boundary
  // (rowId changed), a prepend commit (mounted grew), or a missing sample. A
  // "clean span" is a maximal run of consecutive frames with a valid rowMove and
  // no break inside it. Leg 1 (jerk = second diff) is scored WITHIN clean spans
  // only; Leg 2 (drift) accumulates across everything (excluded frames kept).
  type Step = {
    i: number;
    t: number;
    dt: number;
    rowMove: number;
    scrollDelta: number;
    breakBefore: boolean; // this step cannot chain to the previous (excluded)
  };
  const steps: Step[] = [];
  for (let i = 1; i < frames.length; i++) {
    const a = frames[i - 1];
    const b = frames[i];
    const sameRow = a.rowId != null && a.rowId === b.rowId;
    const haveTops = a.rowTop !== null && b.rowTop !== null;
    const commit = b.mounted > a.mounted; // prepend re-anchor: excluded frame
    if (!sameRow || !haveTops) {
      // No valid rowMove across this boundary — emit no step. The resulting gap
      // in frame indices is detected below and breaks the jerk chain.
      continue;
    }
    steps.push({
      i,
      t: b.t,
      dt: b.t - a.t,
      rowMove: (b.rowTop as number) - (a.rowTop as number),
      scrollDelta: a.scrollTop - b.scrollTop,
      breakBefore: commit, // a commit frame breaks the jerk chain before it
    });
  }
  // A gap in frame indices (a skipped invalid pair — re-pick / missing sample)
  // also breaks the chain: the step after the gap cannot second-difference
  // against a rowMove computed across the excluded region.
  for (let k = 1; k < steps.length; k++) {
    if (steps[k].i !== steps[k - 1].i + 1) steps[k].breakBefore = true;
  }

  // Empirical run velocity (px/ms) from row motion + wall clock only — used as
  // an activity gate and anti-cheat, never in the scorer.
  const totalMove = steps.reduce((acc, s) => acc + s.rowMove, 0);
  const totalDt = steps.reduce((acc, s) => acc + s.dt, 0);
  const velocity = totalDt > 0 ? totalMove / totalDt : 0; // px/ms
  const scrollDir = Math.sign(steps.reduce((acc, s) => acc + s.scrollDelta, 0)); // coarse run direction (up), constant — NOT a per-frame input delta

  // ---- LEG 1: peak |second difference| within clean spans. A span breaks at
  // any step whose breakBefore is set; jerk_k = |rowMove_k − rowMove_{k-1}| is
  // scored only when step k and k-1 are in the same span.
  let peakJerk = 0;
  let jerkCount = 0;
  const jerks: number[] = [];
  let peakAt = -1;
  for (let k = 1; k < steps.length; k++) {
    if (steps[k].breakBefore || steps[k - 1].breakBefore) continue;
    // Activity gate: both frames must have actually scrolled — a paused pair
    // has rowMove≈0 both sides (jerk 0 anyway), but skip to avoid inflating the
    // sample count with dead frames.
    if (
      Math.abs(steps[k].scrollDelta) < 0.5 &&
      Math.abs(steps[k - 1].scrollDelta) < 0.5
    )
      continue;
    const jerk = Math.abs(steps[k].rowMove - steps[k - 1].rowMove);
    jerks.push(jerk);
    jerkCount += 1;
    if (jerk > peakJerk) {
      peakJerk = jerk;
      peakAt = steps[k].i;
    }
  }
  const rmsJerk = jerks.length
    ? Math.sqrt(jerks.reduce((acc, j) => acc + j * j, 0) / jerks.length)
    : 0;

  // ---- LEG 2: peak signed anti-scroll cumulative drift over a trailing
  // horizon. For each end step, walk back DRIFT_HORIZON_MS accumulating only the
  // component of rowMove OPPOSITE the scroll direction (a sustained reversal);
  // scaled forward notches are same-sign and contribute 0.
  let peakDrift = 0;
  for (let end = 0; end < steps.length; end++) {
    let acc = 0;
    let dt = 0;
    for (let start = end; start >= 0 && dt < DRIFT_HORIZON_MS; start--) {
      const anti = -scrollDir * steps[start].rowMove; // >0 == against scroll
      if (anti > 0) acc += anti;
      dt += steps[start].dt;
    }
    if (dt < DRIFT_HORIZON_MS) continue; // not enough history (run start)
    if (acc > peakDrift) peakDrift = acc;
  }

  // Whole-run cumulative anti-scroll drift: the same anti-scroll component as
  // Leg 2 but summed over EVERY step, no trailing window. LOG-ONLY, never gated
  // — the windowed peakDrift is the felt-relevant burst (what you perceive); this
  // is the W4a reference scale: summed-magnitude of ALL anti-scroll motion
  // (non-recovering under-corrections + the reversal halves of recovering
  // frame-late flashes + multi-frame realizations). The ~220px figure quoted
  // earlier was the non-recovering subset only; measured whole-run baseline at
  // e7297849 is WebKit 1042.81 / Chromium 0.00. Windowed can't reach it by
  // construction. The sharp W2 signal: the band should collapse THIS number even
  // if the windowed burst barely moves (RESEARCH/FELT_WHEEL_GATE_METRIC_W4.md).
  let totalDrift = 0;
  for (const step of steps) {
    const anti = -scrollDir * step.rowMove; // >0 == against scroll
    if (anti > 0) totalDrift += anti;
  }

  const commits = frames.filter(
    (f, i) => i > 0 && f.mounted > frames[i - 1].mounted,
  );
  const prependObserved = commits.length > 0;
  const finalMounted = frames[frames.length - 1]?.mounted ?? 0;
  const engine = page.context().browser()?.browserType().name();

  /* eslint-disable no-console */
  console.log(
    `\n=== TRACKPAD-MOMENTUM UPSCROLL GATE (jerk + signed drift, mock jitter-corpus) engine=${engine} ===`,
  );
  console.log(
    `overflow-anchor supported by this engine: ${anchorSupport} (forced 'none' to mirror WKWebView)`,
  );
  console.log(
    `frames=${frames.length} steps=${steps.length} jerk-samples=${jerkCount} commits=${commits.length} swipes=${SWIPES} finalMounted=${finalMounted} velocity=${velocity.toFixed(3)}px/ms`,
  );
  console.log(
    `scored a fetchOlder prepend: ${prependObserved}  (prepend-commit half exercised)`,
  );
  console.log(
    `LEG 1 peak jerk |d2 rect.top|: ${peakJerk.toFixed(2)}px  (gate <= ${MAX_PEAK_JERK_PX})  @frame ${peakAt}  rms=${rmsJerk.toFixed(2)}px`,
  );
  console.log(
    `LEG 2 peak signed drift:       ${peakDrift.toFixed(2)}px over ${DRIFT_HORIZON_MS}ms horizon  (gate <= ${MAX_DRIFT_PX})`,
  );
  console.log(
    `LEG 2 cumulative drift:        ${totalDrift.toFixed(2)}px whole-run  (LOG-ONLY — W4a reference scale, W2 should collapse this)`,
  );
  console.log(
    `LEG 3 rms jerk (chatter):      ${rmsJerk.toFixed(2)}px  (${GATE_RMS_JERK ? `gate <= ${MAX_RMS_JERK_PX}` : "LOG-ONLY — no A/B separation pinned yet"})`,
  );
  console.log(
    "(peak jerk ~0 == smooth row motion; a one-frame spike == felt lurch)",
  );
  console.log("===========================================================\n");
  /* eslint-enable no-console */

  // Sanity: the run actually exercised a meaningful momentum upscroll.
  expect(frames.length).toBeGreaterThan(500);
  expect(finalMounted).toBeGreaterThanOrEqual(80);
  expect(jerkCount).toBeGreaterThan(20);

  // COVERAGE: the run must cross at least one fetchOlder prepend so BOTH lurch
  // sources (CV realization + prepend-commit-under-momentum) are under the gate.
  expect(prependObserved).toBe(true);

  // ANTI-CHEAT: the reading row must actually track the input. A frozen or
  // half-applying scroller has near-zero velocity; assert a real scroll rate
  // (frame-rate invariant — px/ms, not per-frame motion). The floor scales with
  // STEP_PX: a smaller step delivers the same total distance over more events
  // (more per-event pacing overhead), so wall-clock px/ms drops proportionally —
  // a fixed 0.1 floor was pinned at STEP=12 and false-fails a correctly-tracking
  // STEP=2 run. 0.008·STEP_PX ≈ 0.016 at STEP=2 / 0.096 at STEP=12: well above a
  // frozen scroller (~0), well below a real tracking run (STEP=2 measured ~0.07).
  const MIN_VELOCITY = 0.008 * STEP_PX;
  expect(Math.abs(velocity)).toBeGreaterThanOrEqual(MIN_VELOCITY);

  // THE GATE (LEG 1). RED at tip under the WebKit mirror (felt lurches spike
  // jerk well past the threshold); Dawn's engine-order-independent fix produces
  // smooth row motion (jerk ~0) and turns it green on both engines.
  //
  // GUARDRAIL: trustworthy only once a correct writer (Chromium T1.2-green) is
  // confirmed ~0 here under wheel. If jerk can't hold Chromium ~0, invariance is
  // falsified — do NOT relax; fall back to sync-only (option 2). See the header.
  expect(peakJerk).toBeLessThanOrEqual(MAX_PEAK_JERK_PX);
  // Leg 2 (signed drift) — the skip-forever guard. Pinned once the guardrail
  // number lands; asserted here as the second leg of the ratified contract.
  expect(peakDrift).toBeLessThanOrEqual(MAX_DRIFT_PX);
  // Leg 3 (rms-jerk) — the chatter guard. Gated only when the A/B separation is
  // on the record (BUZZ_PERF_GATE_RMS_JERK=1); log-only until then so a run
  // against a correct writer that merely sits at the ~2px discretization floor
  // does not false-red. See §Leg 3 pin criterion in the metric doc.
  if (GATE_RMS_JERK) {
    expect(rmsJerk).toBeLessThanOrEqual(MAX_RMS_JERK_PX);
  }
});
