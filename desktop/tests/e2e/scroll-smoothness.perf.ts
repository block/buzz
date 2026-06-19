import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Scroll-smoothness measurement harness.
 *
 * This is NOT a pass/fail correctness test — it's an instrument. It seeds a
 * busy channel, mounts the full history into the DOM, drives a synthetic
 * fast-wheel scroll up through it, and measures the *main-thread layout work*
 * that scroll triggers — the cost the headless correctness suite cannot feel
 * (it polls only the final position).
 *
 * WHY LAYOUT COST, NOT FRAME CADENCE: under Playwright's headed Chromium there
 * is no vsync throttle, so requestAnimationFrame fires far faster than a real
 * 60Hz display and "frame interval" tells you nothing about paint jank. The
 * honest, engine-agnostic signal is the cumulative layout / style-recalc time
 * Chromium spends servicing the scroll burst, read via CDP Performance metrics.
 * More live DOM rows + per-row reflow on scroll == more layout cost == the jank
 * a user feels. That is exactly the axis virtualization / content-visibility
 * attack, so it's the axis worth a baseline.
 *
 * SCOPE LIMIT (stated honestly): this measures Chromium's main-thread layout
 * cost. It does NOT measure the WKWebView compositor / stale-scrollTop race
 * Sami flagged as the dominant *feel* hazard on the shipped Tauri app — that
 * only reproduces in the real desktop shell and is Tyler's real-wheel pass.
 *
 * Run headed to watch it:
 *   pnpm build && npx playwright test --config=playwright.perf.config.ts --headed
 */

const SEED_ROWS = 600; // a genuinely busy channel, fully mounted
const LINES_PER_ROW = 3;

type Sample = {
  rowCount: number;
  scrollSpan: number;
  frames: number;
};

type Metrics = { layoutMs: number; recalcMs: number; layoutCount: number };

async function readMetrics(
  client: import("@playwright/test").CDPSession,
): Promise<Metrics> {
  const { metrics } = (await client.send("Performance.getMetrics")) as {
    metrics: Array<{ name: string; value: number }>;
  };
  const m = (name: string) => metrics.find((x) => x.name === name)?.value ?? 0;
  return {
    // CDP reports durations in seconds; convert to ms.
    layoutMs: m("LayoutDuration") * 1000,
    recalcMs: m("RecalcStyleDuration") * 1000,
    layoutCount: m("LayoutCount"),
  };
}

test("MEASURE: fast-wheel scroll-up layout cost on a busy un-virtualized timeline", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  await page.evaluate(
    ({ rows, lines }) => {
      // Emit the entire busy channel as live messages so the FULL list mounts
      // into the DOM up front — no dependence on the app's fetch-older
      // pagination (which caps per-request and is a separate axis). This is the
      // un-virtualized DOM we're here to stress.
      for (let i = 0; i < rows; i += 1) {
        const body = Array.from(
          { length: lines },
          (_u, l) => `busy row ${i} line ${l + 1}`,
        ).join("\n");
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: body,
        });
      }
    },
    { rows: SEED_ROWS, lines: LINES_PER_ROW },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");

  // Confirm the list is mounted before measuring. Capture the actual mounted
  // count — we don't assume all SEED_ROWS render (the app may cap/window).
  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.querySelectorAll("[data-message-id]").length > 50;
  });
  await page.waitForTimeout(500); // let live emits settle

  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");

  // Reset to bottom, settle, then snapshot metrics around the scroll burst.
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(100);

  const before = await readMetrics(client);

  const sample = await timeline.evaluate(async (element): Promise<Sample> => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    const startTop = el.scrollTop;
    let frames = 0;

    await new Promise<void>((resolve) => {
      let elapsed = 0;
      let last = performance.now();
      const DURATION_MS = 2_000;
      const PX_PER_FRAME = 50; // brisk human flick

      const step = (now: number) => {
        elapsed += now - last;
        last = now;
        frames += 1;
        el.dispatchEvent(
          new WheelEvent("wheel", {
            deltaY: -PX_PER_FRAME,
            bubbles: true,
            cancelable: true,
          }),
        );
        el.scrollTop = Math.max(0, el.scrollTop - PX_PER_FRAME);
        el.dispatchEvent(new Event("scroll", { bubbles: true }));
        if (elapsed < DURATION_MS && el.scrollTop > 0) {
          requestAnimationFrame(step);
        } else {
          resolve();
        }
      };
      requestAnimationFrame(step);
    });

    return {
      rowCount: el.querySelectorAll("[data-message-id]").length,
      scrollSpan: startTop - el.scrollTop,
      frames,
    };
  });

  const after = await readMetrics(client);
  await client.send("Performance.disable");

  const layoutDelta = after.layoutMs - before.layoutMs;
  const recalcDelta = after.recalcMs - before.recalcMs;
  const layoutCountDelta = after.layoutCount - before.layoutCount;
  const perScroll = sample.frames > 0 ? sample.frames : 1;

  /* eslint-disable no-console */
  console.log("\n=== SCROLL SMOOTHNESS BASELINE (Chromium layout cost) ===");
  console.log(`rows mounted (live DOM):       ${sample.rowCount}`);
  console.log(
    `scroll span covered:           ${Math.round(sample.scrollSpan)}px`,
  );
  console.log(`scroll frames driven:          ${sample.frames}`);
  console.log(`layout time over burst:        ${layoutDelta.toFixed(1)}ms`);
  console.log(`style-recalc time over burst:  ${recalcDelta.toFixed(1)}ms`);
  console.log(`forced layouts (count delta):  ${layoutCountDelta}`);
  console.log(
    `avg layout+recalc per frame:   ${((layoutDelta + recalcDelta) / perScroll).toFixed(3)}ms`,
  );
  console.log("(>~8ms/frame main-thread work is where 120Hz starts to drop)");
  console.log("=========================================================\n");
  /* eslint-enable no-console */

  // Instrument, not a gate — just confirm it exercised the full list.
  expect(sample.rowCount).toBeGreaterThan(100);
  expect(sample.scrollSpan).toBeGreaterThan(500);
});

/**
 * MEASURE: prepend re-render cost — the one event-cost the team had no number
 * for. Steady-state scroll of the bounded 200-row window is compositor-cheap
 * (see the test above). The felt jank, if any, lives at the *prepend*: when
 * older rows land while you're scrolled up, the whole memoized
 * TimelineMessageList re-renders AND the anchor scrollBy fires, all on one
 * main-thread tick. If that tick exceeds the frame budget mid-wheel, the
 * compositor stalls and you feel it.
 *
 * This drives the genuine path: seed older history, scroll off-bottom, then
 * prepend a small batch via the live-event ingest (the same WS path the relay
 * uses), and measure (a) the synchronous wall-time of the tick that flushes
 * the React commit + layout, and (b) the CDP layout/recalc cost attributed to
 * it. It also records whether the anchor held (scrollTop preserved within the
 * row that was under the eye) — a non-restore here is the in-viewport-shift
 * bug, not a perf cost.
 *
 * SCOPE LIMIT: same as above — Chromium main-thread cost, not WKWebView feel.
 */
test("MEASURE: prepend re-render cost while scrolled up (the untested event-cost)", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  // Seed the channel at roughly the real data-window cap (CHANNEL_HISTORY_LIMIT
  // = 200) so the prepend re-renders a full-size list, the worst realistic case.
  const SEED = 200;
  await page.evaluate(
    ({ rows, lines }) => {
      for (let i = 0; i < rows; i += 1) {
        const body = Array.from(
          { length: lines },
          (_u, l) => `seed row ${i} line ${l + 1}`,
        ).join("\n");
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: body,
        });
      }
    },
    { rows: SEED, lines: LINES_PER_ROW },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");

  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.querySelectorAll("[data-message-id]").length > 50;
  });
  await page.waitForTimeout(500);

  // Scroll up off the bottom so the anchor is a mid-list row, not "at-bottom"
  // (the at-bottom path corrects differently). Land roughly in the middle.
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = Math.floor(el.scrollHeight * 0.4);
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(200);

  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");

  const before = await readMetrics(client);
  const scrollTopBefore = await timeline.evaluate(
    (el) => (el as HTMLDivElement).scrollTop,
  );

  // Prepend a small older batch through the live ingest path and time the tick
  // that flushes the resulting React commit + layout + anchor restore.
  const tickMs = await page.evaluate(async (count) => {
    const t0 = performance.now();
    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count,
      lineCount: 3,
      emit: true,
    });
    // Force a layout flush so the synchronous commit + anchor scrollBy + reflow
    // are all accounted before we stop the clock. Two rAFs span the commit and
    // the subsequent paint-prep tick.
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    return performance.now() - t0;
  }, 10);

  const after = await readMetrics(client);
  const scrollTopAfter = await timeline.evaluate(
    (el) => (el as HTMLDivElement).scrollTop,
  );
  const rowCountAfter = await timeline.evaluate(
    (el) => (el as HTMLDivElement).querySelectorAll("[data-message-id]").length,
  );
  await client.send("Performance.disable");

  const layoutDelta = after.layoutMs - before.layoutMs;
  const recalcDelta = after.recalcMs - before.recalcMs;
  const anchorDrift = scrollTopAfter - scrollTopBefore;

  /* eslint-disable no-console */
  console.log("\n=== PREPEND RE-RENDER COST (10 older rows, scrolled up) ===");
  console.log(`rows after prepend (live DOM): ${rowCountAfter}`);
  console.log(`tick wall-time (commit+layout): ${tickMs.toFixed(2)}ms`);
  console.log(`layout time attributed:         ${layoutDelta.toFixed(1)}ms`);
  console.log(`style-recalc time attributed:   ${recalcDelta.toFixed(1)}ms`);
  console.log(
    `anchor drift (scrollTop delta): ${anchorDrift.toFixed(1)}px  (0 = held)`,
  );
  console.log("(tick > ~8ms during active wheel is where 120Hz stalls)");
  console.log("===========================================================\n");
  /* eslint-enable no-console */

  // Instrument, not a gate — just confirm the prepend actually happened.
  expect(rowCountAfter).toBeGreaterThan(50);
});
