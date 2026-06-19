/**
 * Visual regression for the sidebar `MoreUnreadButton` (top variant)
 * overlapping the macOS traffic-light region in the global top chrome.
 *
 * The bug: `position="top"` anchored the pill at `top-0` inside a column
 * starting at window y=0, so it sat inside the 40px chrome strip — where
 * the macOS traffic lights live on the native window.
 *
 * The fix: anchor the top pill to `topChromeInset.top`
 * (`top-(--buzz-top-chrome-height,2.5rem)`) so it lines up with
 * `SidebarContent`'s existing top margin.
 *
 * This spec injects two synthetic pill elements into the live sidebar's
 * relative container — one with the legacy `top-0` className and one with
 * the fixed `top-(--buzz-top-chrome-height,2.5rem)` className — and
 * screenshots each next to the chrome strip so the difference is visible.
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/sidebar-more-unread-overlap";

const LEGACY_TOP_CLASS = "top-0";
const FIXED_TOP_CLASS = "top-(--buzz-top-chrome-height,2.5rem)";

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

  test("before fix: top-0 overlaps traffic-light strip", async ({ page }) => {
    await injectSyntheticPill(page, LEGACY_TOP_CLASS, "synthetic-before");
    const pill = page.getByTestId("synthetic-before");
    await expect(pill).toBeVisible();

    const box = await pill.boundingBox();
    expect(box).not.toBeNull();
    // Pre-fix: pill is anchored at y=0, inside the 40px chrome strip.
    expect(box?.y ?? Number.NaN).toBeLessThan(20);

    // Screenshot the top-left corner of the sidebar with the chrome strip
    // visible above it.
    await page.screenshot({
      path: `${SHOTS}/before-top-0-overlap.png`,
      clip: { x: 0, y: 0, width: 320, height: 120 },
    });
  });

  test("after fix: top-(--buzz-top-chrome-height) clears chrome strip", async ({
    page,
  }) => {
    await injectSyntheticPill(page, FIXED_TOP_CLASS, "synthetic-after");
    const pill = page.getByTestId("synthetic-after");
    await expect(pill).toBeVisible();

    const box = await pill.boundingBox();
    expect(box).not.toBeNull();
    // Post-fix: pill sits below the 40px chrome strip.
    expect(box?.y ?? Number.NaN).toBeGreaterThanOrEqual(40);

    await page.screenshot({
      path: `${SHOTS}/after-top-chrome-inset.png`,
      clip: { x: 0, y: 0, width: 320, height: 120 },
    });
  });
});
