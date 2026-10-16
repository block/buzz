import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { measureAction } from "./perf/metrics";

/**
 * Per-scroll-event main-thread cost of the virtualized timeline.
 *
 * An instrument, not a gate. It parks the reader mid-history in the deep
 * fixture, drives a real CDP wheel burst, and attributes the script, layout
 * and style-recalc time Chromium spends servicing it to the scroll events the
 * burst produced. Work charged per scroll event is work that runs on every
 * tick of a trackpad gesture, so it is the axis that decides whether newly
 * exposed rows paint on time.
 *
 * Run headed to watch it:
 *   pnpm build:e2e && pnpm exec playwright test --config=playwright.perf.config.ts scroll-event-cost
 */

const WHEEL_EVENTS = 60;
const WHEEL_DELTA = -40;

test("MEASURE: main-thread cost per scroll event during a wheel burst", async ({
  page,
}) => {
  await installMockBridge(page, { deepHistoryMessageCount: 1_800 });
  await page.goto("/#/channels/feedf00d-0000-4000-8000-000000000007");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForTimeout(1_000);

  await timeline.evaluate((element) => {
    element.scrollTop = Math.floor(element.scrollHeight / 2);
  });
  await page.waitForTimeout(600);

  await timeline.evaluate((element) => {
    const counter = { scrollEvents: 0 };
    (window as unknown as { __scrollCounter: typeof counter }).__scrollCounter =
      counter;
    element.addEventListener(
      "scroll",
      () => {
        counter.scrollEvents += 1;
      },
      { passive: true },
    );
  });

  const box = await timeline.boundingBox();
  if (!box) throw new Error("timeline has no bounding box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

  const { metrics, wallMs } = await measureAction(page, async () => {
    for (let i = 0; i < WHEEL_EVENTS; i += 1) {
      await page.mouse.wheel(0, WHEEL_DELTA);
      await page.waitForTimeout(8);
    }
    await page.waitForTimeout(200);
  });

  const scrollEvents = await page.evaluate(
    () =>
      (window as unknown as { __scrollCounter: { scrollEvents: number } })
        .__scrollCounter.scrollEvents,
  );
  const per = (value: number) => (value / Math.max(scrollEvents, 1)).toFixed(3);

  /* eslint-disable no-console */
  console.log("\n=== SCROLL EVENT COST (Chromium main thread) ===");
  console.log(`wheel events driven:        ${WHEEL_EVENTS}`);
  console.log(`scroll events observed:     ${scrollEvents}`);
  console.log(`burst wall time:            ${wallMs.toFixed(0)}ms`);
  console.log(
    `script:   ${metrics.scriptMs.toFixed(1)}ms total, ${per(metrics.scriptMs)}ms / scroll event`,
  );
  console.log(
    `layout:   ${metrics.layoutMs.toFixed(1)}ms total, ${per(metrics.layoutMs)}ms / scroll event, ${metrics.layoutCount} layouts (${per(metrics.layoutCount)} / event)`,
  );
  console.log(
    `recalc:   ${metrics.recalcMs.toFixed(1)}ms total, ${per(metrics.recalcMs)}ms / scroll event`,
  );
  console.log(
    `task:     ${metrics.taskMs.toFixed(1)}ms total, ${per(metrics.taskMs)}ms / scroll event`,
  );
  console.log("================================================\n");
  /* eslint-enable no-console */

  expect(scrollEvents).toBeGreaterThan(10);
});
