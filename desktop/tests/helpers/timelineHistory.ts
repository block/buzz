import { expect, type Page } from "@playwright/test";

export const olderWindowRequests = (page: Page) =>
  page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) =>
          entry.command === "get_channel_window" &&
          (entry.payload as { cursor?: unknown } | null)?.cursor != null,
      ).length,
  );

/** A mounted older row is not a page-commit acknowledgement. Wait for the
 * transaction indicator, then leave a quiet interval between reader gestures.
 */
export async function waitForHistorySettled(page: Page) {
  await expect(page.getByTestId("message-timeline-fetching-older")).toHaveCount(
    0,
    { timeout: 10_000 },
  );
  await page.waitForTimeout(250);
}

/** Position the reader, then issue actual input even if already at the boundary.
 * Programmatic positioning alone must not fetch; one gesture gets one window.
 */
export async function startOlderHistory(page: Page) {
  await waitForHistorySettled(page);
  const before = await olderWindowRequests(page);
  const timeline = page.getByTestId("message-timeline");
  await timeline.hover();
  await timeline.evaluate((element) => {
    element.scrollTop = 150;
  });
  await page.mouse.wheel(0, -200);
  await expect.poll(() => olderWindowRequests(page)).toBe(before + 1);
}

export async function pageOlderHistory(page: Page) {
  await startOlderHistory(page);
  await waitForHistorySettled(page);
}
