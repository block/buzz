import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * TRACKPAD-MOMENTUM UPSCROLL GATE (W3) — the merge-blocking, offline sibling of
 * `upscroll-trackpad-live.perf.ts`.
 *
 * Eva's kickoff (thread) localised the WebKit-compensation defect: the T2
 * writer is near-perfect on Chromium and near-ineffective on WKWebView, both
 * settled (68.3% vs 98.9%) and — far worse — under continuous trackpad
 * momentum (39 felt lurches up to 204px per 30-swipe pass). The live probe
 * proved it against real #buzz-bugs but needs staging + a port-forward + a
 * member nsec, so it can never gate a PR merge. This gate reproduces the SAME
 * felt-lurch metric under the SAME momentum actuation on the deterministic
 * mock `jitter-corpus` bridge — no network, greppable in CI, on BOTH engines.
 *
 * WHY THE MOCK COVERS BOTH LURCH SOURCES (the W3 trace, thread):
 *   1. CV-realization lurch — jitter-corpus is the heterogeneous 400-row seed
 *      whose `estimateRowHeight` mismatch is the realization error T1.1/T1.2
 *      already gate; momentum crossing those rows realises them under the
 *      reading row.
 *   2. Prepend-commit-under-momentum — jitter-corpus is 400 rows > the 300
 *      CHANNEL_HISTORY_LIMIT, so `fetchOlder` pages behind an until-cursor.
 *      `useLoadOlderOnScroll` fires the fetch off an IntersectionObserver on a
 *      600px-margin top sentinel — pure geometry, input-cadence-independent —
 *      so momentum triggers it exactly like live. We set
 *      `channelWindowDelayMs` so the page commits several frames LATER, under
 *      continuous momentum, reproducing the commit race rather than an instant
 *      same-frame commit.
 *
 * ENGINE FIDELITY — same mirror as T1.1/T1.2: the shipped WKWebView has no
 * `overflow-anchor`, so we force `overflow-anchor: none` on the scroller and
 * Chromium reproduces the shipped engine. Under Playwright `perf-webkit` this
 * is the real WebKit family. We log `CSS.supports(...)` for the record.
 *
 * WHAT IT MEASURES — identical to the live probe. A per-RAF in-page sampler,
 * independent of input timing, records scrollTop + the tracked centre row's
 * rect.top + mounted count + fetch count every frame. For a solid page,
 *   rowMove(frame) = rect.top delta = scrollTop_before - scrollTop_after
 * so per-frame deviation = rowMove - appliedScroll. Nonzero = content shifted
 * under the viewport that the input did NOT ask for — the felt jump — whether
 * from CV realization, a losing compensation race, or a prepend anchor miss.
 * Prepend-commit frames (mounted grew) are scored separately: the page legitly
 * gains height there, so a one-frame jump is expected and NOT a lurch.
 *
 * TWO GATE SHAPES (Quinn's W4): peak-per-frame catches a single big lurch (a
 * wheel-fight spike); RMS-across-the-run catches SKIP-FOREVER — a too-tight
 * staleness skip-bound that never fires the correction under momentum, showing
 * as a run of small deviations that dodge the peak threshold but sum to drift.
 * The gate asserts BOTH so Dawn's skip-bound is a two-sided constraint.
 *
 * RED-AT-TIP IS LOAD-BEARING (same discipline as T1.2): this gate MUST fail on
 * the contract SHA (`6b9203ca`) under the WebKit mirror. That red is the proof
 * the metric has teeth. A correct engine-order-independent fix (Dawn's W1)
 * turns it green. If a future change greens the tip WITHOUT the fix, this gate
 * is VOID — do not relax thresholds; restore the mirror/corpus so it reds.
 *
 * Run (Chromium):
 *   pnpm build && npx playwright test --config=playwright.perf.config.ts \
 *     upscroll-trackpad-gate
 * Run (WebKit — the engine that actually reds at tip):
 *   npx playwright test --config=playwright.perf-webkit.config.ts \
 *     upscroll-trackpad-gate --project=perf-webkit
 */

// A felt lurch: the tracked reading row jumped in one frame by more than this
// beyond what the input asked, OUTSIDE a prepend-commit frame. Eva's target
// (thread) is the felt threshold from the live probe: |dev|>10px is a jump the
// eye catches. Gate = zero such frames.
const MAX_FELT_LURCHES = 0;
const LURCH_PX = 10;
// Sustained smoothness: fraction of scored frames whose row tracked the input
// within 1px. Eva's target: >=99% on both engines.
const MIN_SMOOTH_FRACTION = 0.99;
// RMS of per-frame deviation across the whole run (px). Quinn's W4 caveat: a
// too-tight staleness skip-bound causes SKIP-FOREVER — the correction never
// fires under momentum, so we're back to main's raw uncompensated shift, but as
// a run of small-to-medium per-notch deviations that each DODGE the peak
// LURCH_PX threshold while summing to large drift. Peak-per-frame is blind to
// that shape; RMS is not. This is the second assertion Quinn asked the mock
// GATE to carry. Placeholder ceiling; PINNED against Dawn's first green
// candidate, same as the lurch count.
const MAX_RMS_DEVIATION_PX = 1.0;
// Keep the tracked row this far (px) from both viewport edges so it stays
// realized across a frame — no straddling-row un-realization artifact.
const SAFE_MARGIN = 60;
// Swipes per run. Enough to cross several fetchOlder pages on the 400-row seed.
const SWIPES = Number(process.env.BUZZ_PERF_SWIPES ?? 30);
// Deferred-commit latency for the mock fetchOlder page, so the prepend commits
// under continuous momentum (mirrors the live ~1s network fetch).
const FETCH_DELAY_MS = Number(process.env.BUZZ_PERF_FETCH_DELAY_MS ?? 1000);

// One macOS-ish swipe: finger ramp (accelerating) + momentum tail (exp decay).
// Byte-for-byte the profile the live probe uses so the two agree.
function swipeDeltas(): number[] {
  const deltas: number[] = [];
  for (let i = 0; i < 12; i++) deltas.push(4 + Math.round((32 * i) / 11));
  let v = 36;
  while (v >= 1) {
    deltas.push(Math.round(v));
    v *= 0.94;
  }
  return deltas; // ~68 events, ~1500px total
}

type Frame = {
  t: number;
  scrollTop: number;
  rowId: string | null;
  rowTop: number | null;
  mounted: number;
  fetch: number;
};

test("GATE: trackpad-momentum upscroll produces zero felt lurches on both engines", async ({
  page,
}) => {
  test.setTimeout(300_000);
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
  // channelWindowDelayMs is read live by the bridge per fetch.
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

  // ---- Per-RAF sampler, in page, independent of input cadence ----
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
      let rowTop: number | null = null;
      if (trackedId) {
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(trackedId)}"]`,
        );
        if (row) {
          const r = row.getBoundingClientRect();
          if (r.top > box.top + margin && r.bottom < box.bottom - margin)
            rowTop = r.top;
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
          rowTop = r ? r.top : null;
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

  // ---- Trusted trackpad-like wheel input via CDP (Chromium) / mouse.wheel ----
  const box = await timeline.boundingBox();
  if (!box) throw new Error("no timeline box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  const isChromium =
    page.context().browser()?.browserType().name() === "chromium";
  const cdp = isChromium ? await page.context().newCDPSession(page) : null;
  await page.mouse.move(cx, cy);

  for (let s = 0; s < SWIPES; s++) {
    const deltas = swipeDeltas();
    for (const d of deltas) {
      if (cdp) {
        await cdp.send("Input.dispatchMouseEvent", {
          type: "mouseWheel",
          x: cx,
          y: cy,
          deltaX: 0,
          deltaY: -d,
          pointerType: "mouse",
        });
      } else {
        await page.mouse.wheel(0, -d);
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

  // ---- Analysis: per-frame deviation of row motion vs applied scroll ----
  type Dev = {
    i: number;
    dev: number;
    applied: number;
    rowMove: number;
    dt: number;
    mountedGrew: boolean;
    fetch: number;
    scrollTop: number;
  };
  const devs: Dev[] = [];
  for (let i = 1; i < frames.length; i++) {
    const a = frames[i - 1];
    const b = frames[i];
    if (!a.rowId || a.rowId !== b.rowId) continue; // re-pick boundary
    if (a.rowTop === null || b.rowTop === null) continue;
    const applied = a.scrollTop - b.scrollTop;
    const rowMove = b.rowTop - a.rowTop;
    devs.push({
      i,
      dev: rowMove - applied,
      applied,
      rowMove,
      dt: b.t - a.t,
      mountedGrew: b.mounted > a.mounted,
      fetch: b.fetch,
      scrollTop: b.scrollTop,
    });
  }

  // Prepend-commit frames legitimately gain height; scored separately.
  const scored = devs.filter((d) => !d.mountedGrew);
  const commits = devs.filter((d) => d.mountedGrew);
  const feltLurches = scored
    .filter((d) => Math.abs(d.dev) > LURCH_PX)
    .sort((x, y) => Math.abs(y.dev) - Math.abs(x.dev));
  const smoothCount = scored.filter((d) => Math.abs(d.dev) <= 1).length;
  const smoothFraction = smoothCount / Math.max(1, scored.length);
  const worstLurch = feltLurches.length ? Math.abs(feltLurches[0].dev) : 0;
  // RMS deviation across ALL scored frames (Quinn's skip-forever signature):
  // a run of sub-LURCH_PX deviations that dodge the peak gate still lifts RMS.
  const rmsDeviation = Math.sqrt(
    scored.reduce((a, d) => a + d.dev * d.dev, 0) / Math.max(1, scored.length),
  );
  // Cumulative unasked drift = signed sum of deviation; a one-directional
  // skip-forever accumulates here even if each frame is small. Diagnostic.
  const cumulativeDrift = scored.reduce((a, d) => a + d.dev, 0);
  const prependObserved = commits.length > 0;
  // Anti-cheat: the reading row must actually track the input. Total input
  // applied over the run must be a meaningful upscroll (a frozen/half-applying
  // scroller has near-zero motion and would false-green on the lurch count).
  const totalApplied = scored.reduce((a, d) => a + d.applied, 0);
  const finalMounted = frames[frames.length - 1]?.mounted ?? 0;

  const engine = page.context().browser()?.browserType().name();
  /* eslint-disable no-console */
  console.log(
    `\n=== TRACKPAD-MOMENTUM UPSCROLL GATE (mock jitter-corpus) engine=${engine} ===`,
  );
  console.log(
    `overflow-anchor supported by this engine: ${anchorSupport} (forced 'none' to mirror WKWebView)`,
  );
  console.log(
    `frames=${frames.length} scored=${scored.length} commits=${commits.length} swipes=${SWIPES} fetchPages=${frames[frames.length - 1]?.fetch ?? 0} finalMounted=${finalMounted}`,
  );
  console.log(
    `felt lurches (|dev|>${LURCH_PX}px, non-commit): ${feltLurches.length}  (gate <= ${MAX_FELT_LURCHES})`,
  );
  console.log(`worst single-frame lurch: ${worstLurch.toFixed(1)}px`);
  console.log(
    `smooth frames (|dev|<=1px): ${smoothCount}/${scored.length} = ${(100 * smoothFraction).toFixed(2)}%  (gate >= ${(100 * MIN_SMOOTH_FRACTION).toFixed(0)}%)`,
  );
  console.log(
    `rms deviation (skip-forever guard): ${rmsDeviation.toFixed(2)}px  (gate <= ${MAX_RMS_DEVIATION_PX})  cumulativeDrift=${cumulativeDrift.toFixed(0)}px`,
  );
  console.log(
    `scored a fetchOlder prepend: ${prependObserved}  (prepend-commit half exercised)`,
  );
  console.log(
    `total upscroll applied (anti-cheat): ${totalApplied.toFixed(0)}px`,
  );
  for (const d of feltLurches.slice(0, 20))
    console.log(
      `  lurch frame=${d.i} dev=${d.dev.toFixed(1)}px applied=${d.applied.toFixed(1)} rowMove=${d.rowMove.toFixed(1)} dt=${d.dt.toFixed(1)}ms fetch=${d.fetch} scrollTop=${d.scrollTop.toFixed(0)}`,
    );
  console.log("===========================================================\n");
  /* eslint-enable no-console */

  // Sanity: the run actually exercised a meaningful momentum upscroll.
  expect(frames.length).toBeGreaterThan(500);
  expect(finalMounted).toBeGreaterThanOrEqual(80);

  // COVERAGE: the run must cross at least one fetchOlder prepend, so BOTH lurch
  // sources (CV realization + prepend-commit-under-momentum) are under the gate.
  // Corpus-structural (400-row seed > 300 limit) — holds on RED tip and GREEN
  // fix alike; a run that never paged is a coverage regression, not a verdict.
  expect(prependObserved).toBe(true);

  // ANTI-CHEAT: the reading row must track the input. A frozen or half-applying
  // scroller (near-zero motion) is caught here well before the lurch count.
  expect(totalApplied).toBeGreaterThan(SWIPES * 200);

  // THE GATE. RED at tip under the WebKit mirror (dozens of felt lurches up to
  // ~200px); Dawn's engine-order-independent fix turns it green on both engines.
  expect(feltLurches.length).toBeLessThanOrEqual(MAX_FELT_LURCHES);
  expect(smoothFraction).toBeGreaterThanOrEqual(MIN_SMOOTH_FRACTION);
  // Quinn's skip-forever guard: sustained sub-lurch under-compensation.
  expect(rmsDeviation).toBeLessThanOrEqual(MAX_RMS_DEVIATION_PX);
});
