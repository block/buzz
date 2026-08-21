import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Sidebar hover intent must warm the hovered channel's message window before
 * the click: dwelling on an unvisited channel row triggers exactly one
 * window fetch, so the subsequent click paints from cache instead of paying
 * the fetch on the switch path. Scrubbing across the row (enter → quick
 * leave) must NOT fetch.
 */

declare global {
  interface Window {
    __CHANNEL_WINDOW_HEAD_FETCH_COUNT__?: number;
    __CHANNEL_WINDOW_HEAD_COMPLETE_COUNT__?: number;
  }
}

async function windowFetchCount(page: import("@playwright/test").Page) {
  return page.evaluate(() => window.__CHANNEL_WINDOW_HEAD_FETCH_COUNT__ ?? 0);
}

async function windowCompleteCount(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => window.__CHANNEL_WINDOW_HEAD_COMPLETE_COUNT__ ?? 0,
  );
}

test("hover dwell prefetches the channel window; scrubbing does not", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  // Seed #random (empty by default) so the warmed cache has a row to paint;
  // recordMockMessage writes to the store without needing a subscription.
  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "random",
      content: "Prefetched row",
      createdAt: Math.floor(Date.now() / 1000) - 300,
    });
  });
  const baseline = await windowFetchCount(page);

  // Scrub: enter and leave immediately — under the dwell, no fetch.
  const random = page.getByTestId("channel-random");
  await random.hover();
  await page.getByTestId("channel-general").hover({ force: true });
  await page.getByTestId("app-sidebar").hover({ position: { x: 4, y: 4 } });
  await page.waitForTimeout(300);
  const afterScrub = await windowFetchCount(page);

  // Dwell: hover and stay past the intent threshold — exactly one fetch for
  // the hovered channel, before any click.
  const completedBeforeDwell = await windowCompleteCount(page);
  await random.hover();
  await expect
    .poll(() => windowFetchCount(page), { timeout: 2_000 })
    .toBe(afterScrub + 1);

  // The warmed cache only exists once the prefetch COMPLETES — a pending or
  // failed prefetch must not pass this spec.
  await expect
    .poll(() => windowCompleteCount(page), { timeout: 2_000 })
    .toBeGreaterThan(completedBeforeDwell);

  // The click then paints the timeline from the warmed cache: rows are
  // visible, not just the header. The subscription-gap refresh may add its
  // own fetch after mount; the paint itself must not wait on one.
  await random.click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-row").first()).toBeVisible();

  // Scrubbing earlier must not have fetched anything beyond the baseline.
  expect(afterScrub).toBe(baseline);
});
