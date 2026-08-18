import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Cold-channel-switch longtask harness.
 *
 * This is the instrument the timeline-virtualization acceptance gate is defined
 * against: it measures the main-thread blocking cost of the FIRST switch into a
 * deep channel (600 seeded messages, windowed to the 300-row ceiling on cold
 * load). main builds every windowed row synchronously on first mount, so that
 * mount is the beachball; virtualization renders only the visible window, so the
 * mount cost is bounded and independent of channel depth.
 *
 * WHY LONGTASKS, NOT LAYOUT METRICS: the felt jank is the main thread being
 * blocked past the ~50ms frame-budget wall during the mount. PerformanceObserver
 * `longtask` entries are exactly the >50ms main-thread tasks the browser itself
 * flags — the honest, engine-level signal for "the UI froze". We report the
 * LONGEST single longtask (the worst freeze) and the TOTAL longtask time across
 * the switch window (the split-task guard axis — many medium tasks can hide the
 * same total cost a single long one would show).
 *
 * WHY 4x CPU THROTTLE: headless Chromium on dev hardware is far faster than the
 * target machines; 4x throttle via CDP brings the mount cost into a regime where
 * a 300-row synchronous build actually crosses the frame-budget wall, so the
 * measurement discriminates. The throttle and machine cancel in the B-vs-header
 * delta gate, so absolute ms are not portable but the delta is.
 *
 * COLD = warm `general` first, then the FIRST switch into `deep-history`. A warm
 * re-entry hits cached render state and does not reproduce the cold mount cost.
 * Each of the 5 runs re-warms `general` so every deep-history switch is cold.
 *
 * SCOPE LIMIT: this measures Chromium main-thread longtasks under throttle. It
 * does NOT measure the WKWebView compositor feel on the shipped Tauri shell —
 * that is a separate real-wheel pass.
 *
 * Run it:
 *   pnpm build && npx playwright test --config=playwright.perf.config.ts \
 *     cold-switch-longtask.perf.ts
 */

const RUNS = 5;
const THROTTLE_RATE = 4;

type RunResult = { longest: number; total: number; count: number };

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[mid - 1] + sorted[mid]) / 2
    : sorted[mid];
}

test("MEASURE: cold-switch longtask cost into deep-history at the 300 ceiling", async ({
  page,
}) => {
  test.setTimeout(120_000);
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Arm a longtask observer that buffers into a window array we can read and
  // reset per run. `buffered: true` catches tasks queued before the read.
  await page.addInitScript(() => {
    const store = window as unknown as { __LONGTASKS__?: number[] };
    store.__LONGTASKS__ = [];
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        store.__LONGTASKS__?.push(entry.duration);
      }
    }).observe({ type: "longtask", buffered: true });
  });
  // addInitScript only applies on the next navigation, so reload to arm it.
  await page.reload();
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      Array.isArray(
        (window as unknown as { __LONGTASKS__?: number[] }).__LONGTASKS__,
      ),
  );

  const client = await page.context().newCDPSession(page);
  await client.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE_RATE });

  const timeline = page.getByTestId("message-timeline");
  const results: RunResult[] = [];

  for (let run = 0; run < RUNS; run += 1) {
    // Warm `general` so the deep-history switch that follows is a cold first
    // entry, not a warm re-render of cached state.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

    // Clear the buffer immediately before the cold switch so only the switch's
    // longtasks are attributed to this run.
    await page.evaluate(() => {
      (window as unknown as { __LONGTASKS__: number[] }).__LONGTASKS__ = [];
    });

    // The cold switch: first entry into the 600-message channel. The cold load
    // windows to the newest 300 and mounts them.
    await page.getByTestId("channel-deep-history").click();
    await expect(page.getByTestId("chat-title")).toHaveText("deep-history");
    await expect(
      page.locator('[data-message-id^="mock-deep-history-"]').first(),
    ).toBeVisible();
    // Let any post-mount longtasks (anchor settle, sticky handoff) flush before
    // reading — they are part of the switch cost.
    await page.waitForTimeout(300);

    const tasks = await page.evaluate(
      () =>
        (window as unknown as { __LONGTASKS__: number[] }).__LONGTASKS__ ?? [],
    );
    results.push({
      longest: tasks.length ? Math.max(...tasks) : 0,
      total: tasks.reduce((sum, d) => sum + d, 0),
      count: tasks.length,
    });
  }

  await client.send("Emulation.setCPUThrottlingRate", { rate: 1 });

  const longests = results.map((r) => r.longest);
  const totals = results.map((r) => r.total);
  const medianLongest = median(longests);
  const minLongest = Math.min(...longests);
  const maxLongest = Math.max(...longests);
  const spread = maxLongest - minLongest;
  const medianTotal = median(totals);

  /* eslint-disable no-console */
  console.log(
    "\n=== COLD-SWITCH LONGTASK BASELINE (deep-history, 300 ceiling) ===",
  );
  console.log(`CPU throttle:                  ${THROTTLE_RATE}x`);
  console.log(`runs:                          ${RUNS}`);
  console.log(
    `per-run longest-longtask (ms): [${longests.map((v) => v.toFixed(1)).join(", ")}]`,
  );
  console.log(
    `per-run total-longtask (ms):   [${totals.map((v) => v.toFixed(1)).join(", ")}]`,
  );
  console.log(
    `per-run longtask count:        [${results.map((r) => r.count).join(", ")}]`,
  );
  console.log(`MEDIAN longest-longtask:       ${medianLongest.toFixed(1)}ms`);
  console.log(
    `  spread (max - min):          ${spread.toFixed(1)}ms (min ${minLongest.toFixed(1)}, max ${maxLongest.toFixed(1)})`,
  );
  console.log(`MEDIAN total-longtask-in-window: ${medianTotal.toFixed(1)}ms`);
  console.log("(>50ms single task is a dropped-frame freeze the user feels)");
  console.log(
    "=================================================================\n",
  );
  /* eslint-enable no-console */

  // Instrument, not a gate — confirm the switch actually exercised the mount
  // under throttle (some longtask work happened on at least one run).
  expect(results.some((r) => r.count > 0)).toBe(true);
});
