import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * UPSCROLL-JITTER GATE (L-E / T1.1) — the RED-today metric the fix train rides on.
 *
 * Tyler's report: scrolling UP a fully-loaded channel is jumpy in tiny
 * variations; scrolling DOWN is smooth. Eva's H1 says why: rows above the
 * opening viewport have NEVER painted, so they sit at `estimateRowHeight()`
 * reserves under `content-visibility: auto`. Scrolling up, each row realizes
 * at its TRUE height the instant it enters the realization band; the delta
 * (true - estimate) shifts content, and (on WKWebView) NOTHING corrects it —
 * one lurch per realization. Down is smooth because `contain-intrinsic-size:
 * auto` has already remembered every passed row's exact size.
 *
 * WHAT THIS MEASURES — MOTION CONSISTENCY, not scrollTop-referenced residual.
 * (This is the T1.1 reframe. The original gate subtracted the REALIZED
 * scrollTop delta, which INCLUDES the fix-writer's compensating `scrollBy`, so
 * the writer cancelled out of the metric and the gate scored raw estimate error
 * R — invariant to whether the fix ran. Dawn found it; Quinn and Eva confirmed
 * the algebra. See the diagnostic below for that residual — it is T3's estimator
 * acceptance number, kept but NON-GATING.)
 *
 * The honest signal: during a steady upscroll a comfortably-visible reading row
 * must move down the viewport by the SAME amount every notch. We actuate a
 * fixed synchronous step (`scrollTop -= STEP`) so the input delta is a known
 * constant every notch — this bypasses Blink's wheel-scaling entirely (a wheel
 * notch can apply 218/220/222… and median-of-run would misread that per-notch
 * scaling as jitter; a fixed step cannot). A perfectly-compensated scroll then
 * moves the reading row by EXACTLY STEP every notch — deviation 0. A broken
 * scroll lurches: rows realizing ABOVE the reading row shove it by the
 * realization delta, which varies notch-to-notch, so its per-notch motion
 * scatters around the run median. The metric is that scatter:
 *
 *   rowMove_i   = after.top - before.top          (reading row's viewport motion; NO scrollTop reference)
 *   deviation_i = | rowMove_i - median(rowMove) |
 *   gate        = peak deviation (worst lurch) and rms deviation (sustained shimmer)
 *
 * WHY THIS ESCAPES THE INVARIANCE that killed the old gate and the co-visible
 * idea: it references the reading row's own motion ACROSS NOTCHES (temporal
 * self-reference), never a neighbouring row (spatial) and never the realized
 * scrollTop. The fix-writer's `scrollBy` moves the row and is NOT subtracted
 * back out, so it survives into rowMove and the metric SEES it. A uniform
 * global offset (which is all compensation is) cancels out of any row-vs-row
 * differential — that is exactly why co-visible and the old residual were
 * blind, and exactly why per-notch-motion-vs-run-median is not.
 *
 * ANTI-CHEAT FLOOR: a frozen or half-applying scroller has near-zero motion
 * variance and would false-green. We assert mean rowMove ≈ STEP so a scroller
 * that does not actually track the input cannot pass.
 *
 * RED-AT-TIP IS A HARD GATE, NOT CEREMONY. median-of-run is only valid if the
 * corpus produces VARYING realization (dispersion); a degenerate constant-R
 * corpus would green a broken writer. The tip run being RED IS the proof that
 * `jitter-corpus` produces that dispersion. If a future corpus edit turns the
 * tip-run green here, this gate is VOID and must fail loudly — see the assert.
 *
 * VALIDITY CONTROL: this metric goes to ~0 only when the reading row tracks the
 * input every notch — i.e. when a correct owned-compensation fix
 * (`overflow-anchor: none` + same-frame `scrollBy(realized − reserved)`) holds
 * it. A synthetic per-notch-varying perfect-comp oracle drives 74/77 notches to
 * EXACTLY STEP (T1.1 bake-off); the real T2 writer re-pins in the same
 * ResizeObserver cycle and closes the remaining synthetic-only outliers.
 *
 * ENGINE FIDELITY: runs under Playwright headed Chromium, which ships
 * `overflow-anchor`. The app ships in WKWebView, which — per Eva's L-C finding
 * (Mari) — has NO `overflow-anchor`: it corrects NOTHING, so every
 * above-viewport realization delta lands raw on the reading position. We FORCE
 * `overflow-anchor: none` on the scroller before measuring so Chromium
 * reproduces the shipped engine, and log `CSS.supports(...)` for the record.
 *
 * Run headed to watch it:
 *   pnpm build && npx playwright test --config=playwright.perf.config.ts \
 *     upscroll-jitter --headed
 */

// Peak per-notch motion deviation we tolerate (px). Above this is a lurch the
// eye catches. RED at tip: baseline peaks ~41px on the heterogeneous corpus.
const MAX_PEAK_DEVIATION_PX = 2.0;
// RMS motion deviation across notches (px) — the sustained micro-jitter floor.
const MAX_RMS_DEVIATION_PX = 0.6;

// Fixed synchronous scroll step per notch (px). Constant by construction — no
// wheel-scaling variance for median-of-run to misread as jitter.
const STEP = 220;
const MAX_STEPS = 80; // cap; the walk now survives fetchOlder re-anchors and
// runs to the true top of history (~77 notches), scoring every paged-in window.
// Keep the tracked row this far (px) from both viewport edges so it stays
// realized across the step — no straddling-row un-realization artifact.
const SAFE_MARGIN = 100;

type StepSample = { rowMove: number; appliedTop: number; residual: number };

type Result = {
  samples: StepSample[];
  reachedTop: boolean;
  rowCount: number;
  // True once a `fetchOlder` prepend landed and was survived mid-run — the
  // gate then keeps scoring notches in the newly paged-in population, so it
  // exercises the pagination half of H1, not just in-window CV realization.
  prependObserved: boolean;
};

// Drive one upscroll run and collect per-notch samples. `actuate` selects the
// input mode: "sync" (fixed STEP, the gate) or "wheel" (Blink-scaled, printed
// as a non-gating felt-mode diagnostic — Tyler's real input is a wheel).
async function measure(
  page: import("@playwright/test").Page,
  actuate: "sync" | "wheel",
): Promise<Result> {
  const timeline = page.getByTestId("message-timeline");
  // Re-pin to the true bottom so everything above is unpainted (at estimate).
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(150);
  await timeline.evaluate((element) => {
    (element as HTMLElement).style.overflowAnchor = "none";
  });
  await page.waitForTimeout(50);
  await timeline.hover();

  const samples: StepSample[] = [];
  let reachedTop = false;
  let prependObserved = false;
  // Mounted-row count before the walk begins — the baseline a prepend grows.
  const mountedAtRunStart = await timeline.evaluate(
    (el) => (el as HTMLDivElement).querySelectorAll("[data-message-id]").length,
  );

  for (let step = 0; step < MAX_STEPS; step += 1) {
    // Pick a row comfortably inside the viewport (SAFE_MARGIN from both edges)
    // so it stays realized across the notch. Capture its top + scrollTop BEFORE.
    const before = await timeline.evaluate((element, margin: number) => {
      const el = element as HTMLDivElement;
      const box = el.getBoundingClientRect();
      const rows = el.querySelectorAll<HTMLElement>("[data-message-id]");
      for (const row of rows) {
        const rect = row.getBoundingClientRect();
        if (rect.top > box.top + margin && rect.bottom < box.bottom - margin) {
          return {
            id: row.dataset.messageId ?? null,
            top: rect.top,
            scrollTop: el.scrollTop,
          };
        }
      }
      return { id: null, top: 0, scrollTop: el.scrollTop };
    }, SAFE_MARGIN);

    if (before.scrollTop <= 0) {
      reachedTop = true;
      break;
    }

    // Actuate one notch upward.
    if (actuate === "sync") {
      await timeline.evaluate((element, s: number) => {
        const el = element as HTMLDivElement;
        el.scrollTop = Math.max(0, el.scrollTop - s);
        el.dispatchEvent(new Event("scroll", { bubbles: true }));
      }, STEP);
    } else {
      await page.mouse.wheel(0, -STEP);
    }
    // Settle two frames so realization + any writer compensation land before we
    // read positions back.
    await timeline.evaluate(
      () =>
        new Promise<void>((r) =>
          requestAnimationFrame(() => requestAnimationFrame(() => r())),
        ),
    );

    if (!before.id) {
      // No safe-band row this step (rare) — nudged already, try the next.
      continue;
    }

    const after = await timeline.evaluate(
      (element, args: { id: string; margin: number }) => {
        const el = element as HTMLDivElement;
        const box = el.getBoundingClientRect();
        const mounted = el.querySelectorAll("[data-message-id]").length;
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(args.id)}"]`,
        );
        if (!row)
          return {
            top: null,
            inSafeBand: false,
            scrollTop: el.scrollTop,
            mounted,
          };
        const rect = row.getBoundingClientRect();
        return {
          top: rect.top,
          inSafeBand:
            rect.top > box.top + args.margin &&
            rect.bottom < box.bottom - args.margin,
          scrollTop: el.scrollTop,
          mounted,
        };
      },
      { id: before.id, margin: SAFE_MARGIN },
    );

    const appliedTop = before.scrollTop - after.scrollTop; // realized px moved up
    if (appliedTop <= 0) {
      // scrollTop did not decrease. Two causes: (a) we reached the top of
      // history (scrollTop ~ 0) — the real terminator; or (b) a `fetchOlder`
      // prepend just landed ~CHANNEL_HISTORY_LIMIT older rows ABOVE the
      // viewport, so scrollTop jumped UP to preserve the visible content — a
      // RE-ANCHOR, not the top. Distinguish by mounted-count: a prepend grows
      // it. On a re-anchor we skip scoring this step (its motion is the
      // prepend jump, not a fixed STEP notch — scoring it would poison the
      // metric with a one-off multi-thousand-px move) and CONTINUE; the next
      // iteration re-baselines a fresh safe-band row in the newly paged-in
      // window, so scoring resumes ACROSS the prepend. This is what makes the
      // gate SCORE at least one prepend per run (T1.2) rather than terminate
      // at the sentinel band — closing the pagination-coverage gap Quinn and
      // Dawn flagged. Both the writer (GREEN) and the bare estimator (RED)
      // survive the re-anchor, so RED still fails on the cold-realization
      // jitter of the paged-in rows and GREEN still holds them to STEP.
      if (after.mounted > mountedAtRunStart && after.scrollTop > 1) {
        prependObserved = true;
        continue;
      }
      reachedTop = after.scrollTop <= 0;
      break;
    }
    if (after.top === null || !after.inSafeBand) {
      // Tracked row left the safe band — skip scoring, keep scrolling.
      continue;
    }
    const rowMove = after.top - before.top; // viewport motion — the gated signal
    // NON-GATING diagnostic: motion minus REALIZED scrollTop delta = raw
    // estimate error R (comp-invariant). This is T3's estimator acceptance
    // number, printed for the record; it is NOT the gate.
    const residual = rowMove - appliedTop;
    samples.push({ rowMove, appliedTop, residual });
  }

  return {
    samples,
    reachedTop,
    rowCount: await timeline.evaluate(
      (el) =>
        (el as HTMLDivElement).querySelectorAll("[data-message-id]").length,
    ),
    prependObserved,
  };
}

function stats(samples: StepSample[]) {
  const moves = samples.map((s) => s.rowMove);
  const sorted = moves.slice().sort((a, b) => a - b);
  const median = sorted.length ? sorted[Math.floor(sorted.length / 2)] : 0;
  const dev = moves.map((m) => Math.abs(m - median));
  const peakDev = dev.length ? Math.max(...dev) : 0;
  const rmsDev = dev.length
    ? Math.sqrt(dev.reduce((a, d) => a + d * d, 0) / dev.length)
    : 0;
  const meanMove = moves.length
    ? moves.reduce((a, m) => a + m, 0) / moves.length
    : 0;
  // Old scrollTop-referenced residual (= estimate error R) — non-gating diag.
  const resAbs = samples.map((s) => Math.abs(s.residual));
  const resPeak = resAbs.length ? Math.max(...resAbs) : 0;
  const resRms = resAbs.length
    ? Math.sqrt(resAbs.reduce((a, r) => a + r * r, 0) / resAbs.length)
    : 0;
  return { median, peakDev, rmsDev, meanMove, resPeak, resRms };
}

test("GATE: upscroll motion consistency stays below the realization-jitter threshold", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // The `jitter-corpus` channel is pre-seeded (e2eBridge.ts) with 400
  // heterogeneous rows in its mock store — the same reliable cold-load path
  // scroll-history.spec.ts uses, no dependence on live-emit timing.
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

  // Log native-anchoring support for this engine (measure() forces it off).
  const anchorSupport = await page.evaluate(() =>
    typeof CSS !== "undefined" && typeof CSS.supports === "function"
      ? CSS.supports("overflow-anchor", "auto")
      : false,
  );

  // GATED run: fixed synchronous step (deterministic input, no Blink scaling).
  const result = await measure(page, "sync");
  const s = stats(result.samples);

  // NON-GATING felt-mode diagnostic: same metric under a real wheel. Tyler's
  // input is a wheel; we want the felt number on the record every run even
  // though Blink's per-notch scaling makes it unsafe to gate on.
  const wheelResult = await measure(page, "wheel");
  const w = stats(wheelResult.samples);

  /* eslint-disable no-console */
  console.log(
    `overflow-anchor supported by this engine: ${anchorSupport} ` +
      `(forced to 'none' on the scroller to mirror shipped WKWebView)`,
  );
  console.log("\n=== UPSCROLL JITTER GATE (motion consistency, Chromium) ===");
  console.log(`rows mounted (live DOM):     ${result.rowCount}`);
  console.log(`steps measured (sync):       ${result.samples.length}`);
  console.log(`reached top of history:      ${result.reachedTop}`);
  console.log(
    `scored a fetchOlder prepend: ${result.prependObserved}  (T1.2: pagination half exercised)`,
  );
  console.log(
    `median per-notch motion:     ${s.median.toFixed(1)}px  (STEP=${STEP})`,
  );
  console.log(
    `peak motion deviation:       ${s.peakDev.toFixed(2)}px  (gate <= ${MAX_PEAK_DEVIATION_PX})`,
  );
  console.log(
    `rms motion deviation:        ${s.rmsDev.toFixed(2)}px  (gate <= ${MAX_RMS_DEVIATION_PX})`,
  );
  console.log(
    `mean per-notch motion:       ${s.meanMove.toFixed(1)}px  (anti-cheat: ~= ${STEP})`,
  );
  console.log("--- non-gating diagnostics ---");
  console.log(
    `estimate-error R (old resid):peak=${s.resPeak.toFixed(2)}px rms=${s.resRms.toFixed(2)}px  (T3 estimator acceptance number)`,
  );
  console.log(
    `wheel-actuated (felt mode):  peak-dev=${w.peakDev.toFixed(2)}px rms-dev=${w.rmsDev.toFixed(2)}px over ${wheelResult.samples.length} steps`,
  );
  console.log("(0 == the reading row tracked the input exactly every notch)");
  console.log("===========================================================\n");
  /* eslint-enable no-console */

  // Sanity: the run actually exercised a meaningful upscroll. Cold-load windows
  // the 400-row seed to the newest ~100 rows (CHANNEL_HISTORY_LIMIT), all in the
  // DOM (de-virtualized), so ~100 mounted rows is the expected window.
  expect(result.rowCount).toBeGreaterThanOrEqual(80);
  expect(result.samples.length).toBeGreaterThan(8);

  // COVERAGE (T1.2): the scored run must cross at least one `fetchOlder`
  // prepend. Upscroll jitter has TWO sources (Eva's H1 + Quinn's seed trace):
  // CV-skipped rows in the opening window that realize on entry, AND cold rows
  // paged in above the viewport when scrollback fires near the top. The gate
  // measures felt motion whatever the cause, but if the run terminated at the
  // sentinel band it would only ever certify the first source. We assert it
  // survived a re-anchor and kept scoring so BOTH sources are under the gate —
  // pass or fail. This is corpus-structural (400-row seed > 300 limit), so it
  // holds on the RED baseline and the GREEN fix alike; a run that stops before
  // scoring a prepend is a coverage regression, not a jitter verdict.
  expect(result.prependObserved).toBe(true);

  // ANTI-CHEAT: the reading row must actually track the input — a frozen or
  // half-applying scroller (near-zero motion, would false-green on deviation)
  // is caught here because its mean motion falls well below STEP.
  expect(s.meanMove).toBeGreaterThan(STEP * 0.75);

  // THE GATE — per-notch motion consistency. RED at tip (~41px peak / ~23px rms
  // on jitter-corpus); a correct owned-compensation fix (T2 writer) drives the
  // reading row to STEP every notch and turns it green.
  //
  // ⚠️ RED-AT-TIP IS LOAD-BEARING: median-of-run is only valid while the corpus
  // produces VARYING realization. This gate failing at tip IS the proof of that
  // dispersion. If a future corpus edit makes the tip run PASS here WITHOUT a
  // compensation fix, this gate is VOID — do not relax the thresholds; restore a
  // heterogeneous corpus so the gate reds again, or the whole metric is moot.
  expect(s.peakDev).toBeLessThanOrEqual(MAX_PEAK_DEVIATION_PX);
  expect(s.rmsDev).toBeLessThanOrEqual(MAX_RMS_DEVIATION_PX);
});
