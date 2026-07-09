import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Regression gate for the markdown parse cache (shared/ui/markdown/nodeCache).
 *
 * The timeline's per-channel-switch remount used to re-run every row's
 * react-markdown parse; the cache makes warm re-entries pure element reuse.
 * This spec asserts the deterministic invariant behind that win: a warm
 * channel switch performs ZERO fresh parses. Unlike the wall-clock
 * benchmark (warm-switch-markdown.perf.ts, instrument-only), this is
 * machine-independent and safe to gate CI on — if someone disconnects
 * MarkdownInner from the cache or breaks the key, this fails.
 */

async function parseCount(page: import("@playwright/test").Page) {
  return page.evaluate(() => window.__BUZZ_E2E_MD_PARSE_COUNT__?.() ?? -1);
}

async function settleChannel(
  page: import("@playwright/test").Page,
  title: string,
) {
  await expect(page.getByTestId("chat-title")).toHaveText(title);
  await expect(page.locator('[data-render-pending="true"]')).toHaveCount(0);
  await expect(
    page.getByTestId("message-timeline").locator("[data-message-id]").first(),
  ).toBeVisible();
  // Let any trailing deferred commits flush before reading the counter.
  await page.waitForTimeout(300);
}

test("warm channel switches trigger zero fresh markdown parses", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_MD_PARSE_COUNT__ === "function",
  );

  // Cold visits populate the query caches and the markdown node cache.
  await page.getByTestId("channel-deep-history").click();
  await settleChannel(page, "deep-history");
  await page.getByTestId("channel-general").click();
  await settleChannel(page, "general");

  const afterCold = await parseCount(page);
  // Sanity: the cold visits really parsed rows (counter is wired up).
  expect(afterCold).toBeGreaterThan(10);

  // Warm switches: every row must be a cache hit.
  let previous = afterCold;
  for (let round = 0; round < 2; round += 1) {
    await page.getByTestId("channel-deep-history").click();
    await settleChannel(page, "deep-history");
    const inDeepHistory = await parseCount(page);
    expect(
      inDeepHistory - previous,
      `warm switch into deep-history (round ${round}) re-parsed markdown`,
    ).toBe(0);

    await page.getByTestId("channel-general").click();
    await settleChannel(page, "general");
    const inGeneral = await parseCount(page);
    expect(
      inGeneral - inDeepHistory,
      `warm switch into general (round ${round}) re-parsed markdown`,
    ).toBe(0);
    previous = inGeneral;
  }
});
