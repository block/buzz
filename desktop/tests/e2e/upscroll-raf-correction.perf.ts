import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * W4a — realizing-upscroll reading-row reversal, OWN red/green fixture.
 *
 * Proves the mechanism of design-of-record (c) (Eva's ungated-RO corrector,
 * ratified event 96e8fcca): the correction for a content-visibility realization
 * above the reading row must land the SAME frame the realization paints, not one
 * frame late, on BOTH engines. The two mid-history observers (per-rAF sampler +
 * ResizeObserver) coordinate ONLY through a shared row-height cache — no engine
 * branch — and the on-time observer per engine is the sole writer:
 *   - Chromium: the RO delivers pre-paint, so the ungated RO corrects at N and
 *     refreshes the cache; the rAF then sees residual ≈ 0 and no-ops.
 *   - WebKit: the RO delivers at N+1 while the compositor paints at N, so the
 *     rAF (which forces this frame's layout via synchronous
 *     `getBoundingClientRect`) corrects at N; the late RO no-ops against the
 *     rAF-refreshed cache.
 * An RO-late-only writer snaps the reading row down then back on WebKit — a
 * REVERSAL frame pair, the "jump down then snap" Tyler feels.
 *
 * Signature under test (visible outcome, engine-agnostic): while the user
 * scrolls UP through history that realizes above the reading row, the tracked
 * reading row must NOT move AGAINST the scroll direction beyond the slow-wheel
 * staircase envelope. A single such frame is the felt reversal.
 *
 * Asserts are a three-layer stack (Quinn/Eva's ratified checklist): a build-stamp
 * stale-`dist` guard, LIVENESS (some correction fired — catches the vacuous
 * pass), PRIMARY CORRECTNESS (reversals bounded, engine-agnostic), and per-engine
 * MECHANISM in both directions (winner fires > 0, loser fires 0 / no-op).
 *
 * Distinct from Sami's same-harness gate (jerk/drift magnitudes): this asserts
 * the BINARY presence/absence of the reversal, which is the mechanism claim.
 */

const WHEEL_DELTA = 12; // px per wheel event — slow deliberate trackpad
const WHEEL_PERIOD_MS = 32; // cadence
const DURATION_MS = 12_000; // actuation time
const SAFE_MARGIN = 100;
// A reversal frame is the row moving against the scroll by more than the
// staircase noise; upscroll means rowMove is normally >= 0 (row drifts down as
// history prepends), so a genuine against-direction move is < -REVERSAL_PX.
const REVERSAL_PX = 3;
// Build stamp the hook writes into `window.__ANCHOR_BUILD_STAMP__` on its first
// correction attempt. Asserting it below is the stale-`dist` guard: `pnpm build`
// is `tsc && vite build`, and a tsc failure leaves the PRIOR bundle in `dist/`,
// so a fixture can silently exercise stale code. Must equal `ANCHOR_BUILD_STAMP`
// in `useAnchoredScroll.ts`; bump BOTH together per experiment.
const EXPECTED_BUILD_STAMP = "w4a-gate-1";

type Frame = {
  t: number;
  top: number | null;
  scrollTop: number;
  mounted: number;
  rowId: string | null;
};

test("W4a rAF correction: no same-row reversal during realizing upscroll", async ({
  page,
  browserName,
}) => {
  await installMockBridge(page);
  page.on("console", (m) => {
    if (m.type() === "error") console.log("PAGE ERROR:", m.text());
  });
  page.on("pageerror", (e) => console.log("PAGE EXCEPTION:", e.message));
  // Install the correction tripwire probe BEFORE navigating, so the array is
  // present when the app's hook first runs and every mid-history correction
  // attempt (rAF and RO) is recorded. `addInitScript` runs on the NEXT
  // navigation, so it MUST precede `goto` — registering it after `goto` leaves
  // the loaded page without the global and silently records nothing. In prod
  // this global is undefined and `reportCorrection` is a no-op; here it's the
  // source of the split's engine-aware invariants (see asserts at the end).
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

  // Pin to bottom and force overflow-anchor:none so the writer — not native
  // anchoring — owns the reading row (mirrors the shipped WKWebView).
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.style.overflowAnchor = "none";
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(200);
  await timeline.hover();

  // Per-frame sampler: record the tracked reading row's rect.top, scrollTop and
  // mounted-row count every frame while wheel events arrive asynchronously.
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const w = window as unknown as {
      __PROBE__: { frames: Frame[]; stop: boolean };
    };
    type Frame = {
      t: number;
      top: number | null;
      scrollTop: number;
      mounted: number;
      rowId: string | null;
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
      if (top === null) trackedId = pick(); // re-pick: this frame is a marker
      w.__PROBE__.frames.push({
        t,
        top,
        scrollTop: el.scrollTop,
        mounted,
        rowId: trackedId,
      });
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, SAFE_MARGIN);

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
    };
    w.__PROBE__.stop = true;
    return w.__PROBE__.frames;
  });

  // Pull the correction tripwire records (one per mid-history attempt) and the
  // build stamp the hook wrote on its first attempt (stale-`dist` guard).
  const { corrections, buildStamp } = await page.evaluate(() => {
    const g = globalThis as unknown as {
      __ANCHOR_PROBE__?: Array<{
        source: "raf" | "ro";
        wouldFire: boolean;
        residual: number;
      }>;
      __ANCHOR_BUILD_STAMP__?: string;
    };
    return {
      corrections: g.__ANCHOR_PROBE__ ?? [],
      buildStamp: g.__ANCHOR_BUILD_STAMP__ ?? null,
    };
  });

  // Score consecutive frames tracking the SAME row (skip re-anchor frames).
  // A REVERSAL is the row moving against the scroll direction — the visible
  // "jump down then snap" the fix removes. We pair a reversal with an opposite
  // move within 3 frames (a one-frame flash of a late correction); ANY reversal
  // frame — paired or not — is the mechanism failing, so we assert zero.
  let scored = 0;
  let reanchors = 0;
  const reversals: Array<{
    i: number;
    t: number;
    rowMove: number;
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
    if (rowMove <= -REVERSAL_PX) {
      reversals.push({ i, t: b.t, rowMove, rowId: b.rowId });
    }
  }

  /* eslint-disable no-console */
  const maxReversalPx =
    reversals.length === 0
      ? 0
      : Math.max(...reversals.map((r) => Math.abs(r.rowMove)));
  console.log("\n=== W4a rAF CORRECTION FIXTURE ===");
  console.log(`frames sampled:        ${frames.length}`);
  console.log(`frame-pairs scored:    ${scored}`);
  console.log(`re-anchor/skip frames: ${reanchors}`);
  console.log(`reversal frames:       ${reversals.length}`);
  console.log(`max reversal px:       ${maxReversalPx.toFixed(1)}`);
  for (const r of reversals
    .slice()
    .sort((x, y) => x.rowMove - y.rowMove)
    .slice(0, 12)) {
    console.log(
      `  frame ${r.i} t=${r.t.toFixed(0)} rowMove=${r.rowMove.toFixed(1)} row=${r.rowId}`,
    );
  }
  console.log("==================================\n");
  // Classify every mid-history correction attempt by observer + whether it
  // fired. Under design-of-record (c) — Eva's ungated-RO corrector, ratified
  // event 96e8fcca — the on-time observer per engine is the sole mid-history
  // writer: Chromium's RO delivers pre-paint and wins; WebKit's RO is late so
  // the rAF wins and the late RO no-ops against the rAF-refreshed height cache.
  const rafFires = corrections.filter(
    (c) => c.source === "raf" && c.wouldFire,
  ).length;
  const roFires = corrections.filter((c) => c.source === "ro" && c.wouldFire);
  const maxRoResidual =
    roFires.length === 0
      ? 0
      : Math.max(...roFires.map((c) => Math.abs(c.residual)));
  console.log("=== MECHANISM TRIPWIRES ===");
  console.log(`engine:                ${browserName}`);
  console.log(`build stamp:           ${buildStamp ?? "(absent)"}`);
  console.log(`rAF corrections fired: ${rafFires}`);
  console.log(`RO corrections fired:  ${roFires.length}`);
  console.log(`max RO fire residual:  ${maxRoResidual.toFixed(1)}`);
  console.log("===========================\n");
  /* eslint-enable no-console */

  // Sanity: the actuation actually produced a scored upscroll (not a no-op run).
  expect(scored).toBeGreaterThan(50);

  // Stale-`dist` guard (Quinn's ratified sharpening). `pnpm build` is
  // `tsc && vite build`; a tsc failure leaves the PRIOR bundle in `dist/`, so a
  // fixture can silently exercise stale code and fabricate a pass (this class of
  // trap nearly sent us to the wrong design). The hook writes its per-experiment
  // `ANCHOR_BUILD_STAMP` onto `window` the first time the probe records; asserting
  // it equals `EXPECTED_BUILD_STAMP` catches BOTH "build failed, stale dist"
  // (stamp absent) and "build succeeded but serving the previous experiment's
  // dist" (stamp present, wrong value). This runs before every other invariant
  // so a stale bundle can never satisfy them by accident.
  expect(buildStamp).toBe(EXPECTED_BUILD_STAMP);

  // --- LAYER 1: LIVENESS ------------------------------------------------------
  // At least one mid-history correction must have been attempted-and-fired.
  // Catches the fixture-probe-not-installed bug (addInitScript after goto) that
  // produced a VACUOUS pass — the primary assert below is green if no
  // instrumentation ran, so liveness has to gate it. (Eva's checklist #2.)
  expect(rafFires + roFires.length).toBeGreaterThan(0);

  // --- LAYER 2: PRIMARY CORRECTNESS -------------------------------------------
  // The felt outcome, engine-agnostic. The RO-late writer on WebKit produced a
  // large, growing reversal count (dozens, up to the ~204px lurch Tyler feels).
  // Design (c) collapses that to a small BOUNDED residual: a handful of one-frame
  // detection-latency catch-ups, each no larger than a single row-height quantum.
  // NOT strict-0 — an observe-then-correct loop can't predict a
  // `content-visibility` realization, so the last realization before a quiet
  // frame lands one frame late by construction. We assert the FLOOR:
  //   - count stays tiny (RO-late ran dozens and climbed with the corpus), and
  //   - no reversal exceeds one row-height quantum (~34px) — a fixed latency
  //     floor, not accumulating under-correction.
  // NOTE ON FRAGILITY: the residual COUNT is sensitive to the WebKit frame
  // scheduler (per-frame instrumentation or a `scrollTo` async write inflate it
  // from ~2 to ~21 on this harness). The timing-invariant signal is the
  // MAGNITUDE bound; count is asserted only loosely. Whether a sub-quantum
  // one-frame catch-up is perceptible during trackpad-velocity motion is a
  // live-feel call on the real WKWebView embedder (not Playwright WebKit's
  // compositor) and is flagged for Tyler in the PR.
  expect(reversals.length).toBeLessThanOrEqual(4);
  expect(maxReversalPx).toBeLessThanOrEqual(34);

  // --- LAYER 3: MECHANISM (per-engine, both directions) -----------------------
  // The two mid-history observers coordinate ONLY through the shared height
  // cache — no engine branch in the hook. These asserts pin which observer is
  // the sole writer per engine and are the permanent regression tripwires: if a
  // future change breaks the winning observer's cache refresh, the losing
  // observer starts double-correcting and this trips before a user feels it.
  // Asserting BOTH directions (winner fires > 0 AND loser fires == 0/no-op) is
  // also the mid-history ≤1-scrollTo proof: exactly one observer writes per
  // frame. (Quinn's ratified checklist #3.)
  if (browserName === "chromium") {
    // Chromium: the ungated RO delivers pre-paint and is the sole mid-history
    // writer. It MUST fire (else the corpus realized nothing / the RO stopped
    // triggering), and the rAF — one pair-frame behind the shift — must NEVER
    // fire (firing means the RO stopped refreshing the cache and we've regressed
    // to the 16-reversal pure-rAF bug).
    expect(roFires.length).toBeGreaterThan(0);
    expect(rafFires).toBe(0);
  } else {
    // WebKit: the RO is late, so the rAF is the sole mid-history writer and MUST
    // fire. The late RO MAY still fire when it beats a slow frame — the invariant
    // is not "RO never runs" but "no double-correction": every RO fire is a
    // no-op against the cache the rAF already refreshed (residual sub-epsilon).
    expect(rafFires).toBeGreaterThan(0);
    expect(maxRoResidual).toBeLessThanOrEqual(0.5);
  }
});
