import type { Locator, Page } from "@playwright/test";

/**
 * Step-scroll the (virtualized) inbox list until `text` renders.
 *
 * Rows outside the viewport are not in the DOM, so a single jump-to-bottom
 * can skip past a mid-list row entirely. Scroll one viewport at a time; at
 * the bottom, give cursor pagination a beat to extend the list, and wrap
 * back to the top when it doesn't (live polling can prepend rows and shift
 * earlier content out of the rendered window).
 */
export async function scrollInboxListToText(
  page: Page,
  list: Locator,
  text: string,
  timeoutMs = 90_000,
) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await list.getByText(text).count()) > 0) return;
    const atBottom = await list.evaluate((el) => {
      const bottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 4;
      if (!bottom) el.scrollTop += el.clientHeight * 0.8;
      return bottom;
    });
    if (atBottom) {
      const heightBefore = await list.evaluate((el) => el.scrollHeight);
      await page.waitForTimeout(1_000);
      const heightAfter = await list.evaluate((el) => el.scrollHeight);
      if (heightAfter <= heightBefore) {
        await list.evaluate((el) => {
          el.scrollTop = 0;
        });
      }
    }
    await page.waitForTimeout(150);
  }
  throw new Error(`"${text}" never rendered in the inbox list`);
}
