/**
 * Visual regression for the sidebar `MoreUnreadButton` (top variant)
 * overlapping the macOS traffic-light region in the global top chrome.
 *
 * The old bug: `position="top"` anchored the pill at `top-0` inside a column
 * starting at window y=0, so it sat inside the 40px chrome strip where the
 * macOS traffic lights live on the native window.
 *
 * The current layout keeps the global top chrome in normal flow above the
 * sidebar row, so `top-0` inside the sidebar starts below the traffic-light
 * strip.
 *
 * This spec injects a synthetic pill into the live sidebar's relative
 * container and screenshots it next to the chrome strip so the positioning is
 * visible.
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/sidebar-more-unread-overlap";

const TOP_CLASS = "top-0";

const PILL_BASE =
  "pointer-events-none absolute inset-x-0 z-10 flex justify-center py-1";
const PILL_INNER_HTML = `
  <button
    type="button"
    class="pointer-events-auto inline-flex items-center gap-1 rounded-full bg-destructive px-2.5 py-0.5 text-xs font-medium text-destructive-foreground shadow-sm"
  >
    <span>↑ 12 new</span>
  </button>
`;

async function injectSyntheticPill(
  page: import("@playwright/test").Page,
  topClass: string,
  testId: string,
) {
  await page.evaluate(
    ({ topClass, base, html, testId }) => {
      const container = document.querySelector(
        '[data-testid="app-sidebar-scroll-anchor"]',
      ) as HTMLElement | null;
      if (!container) throw new Error("sidebar scroll anchor not found");

      // Remove any prior injection so we can screenshot variants in sequence.
      container
        .querySelectorAll("[data-synthetic-more-unread]")
        .forEach((el) => {
          el.remove();
        });

      const pill = document.createElement("div");
      pill.dataset.syntheticMoreUnread = "true";
      pill.dataset.testid = testId;
      pill.className = `${base} ${topClass}`;
      pill.innerHTML = html;
      container.appendChild(pill);
    },
    { topClass, base: PILL_BASE, html: PILL_INNER_HTML, testId },
  );
}

test.describe("sidebar MoreUnreadButton top chrome overlap", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("app-sidebar")).toBeVisible();
  });

  test("top pill clears the in-flow traffic-light strip", async ({ page }) => {
    await injectSyntheticPill(page, TOP_CLASS, "synthetic-top");
    const pill = page.getByTestId("synthetic-top");
    await expect(pill).toBeVisible();

    const box = await pill.boundingBox();
    expect(box).not.toBeNull();
    // The pill is anchored at the top of the sidebar row, below the 40px
    // in-flow chrome strip.
    expect(box?.y ?? Number.NaN).toBeGreaterThanOrEqual(40);

    await page.screenshot({
      path: `${SHOTS}/top-pill-below-in-flow-chrome.png`,
      clip: { x: 0, y: 0, width: 320, height: 120 },
    });
  });
});
