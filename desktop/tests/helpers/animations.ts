import type { Page } from "@playwright/test";

/**
 * Wait for all in-flight CSS/Web animations on the page to finish.
 *
 * Radix UI components animate in via CSS transitions. Playwright's
 * `toBeVisible()` resolves mid-animation, producing greyed-out or
 * partially-rendered screenshots. Call this before any `page.screenshot()`
 * or `locator.screenshot()` to guarantee a fully-rendered frame.
 */
export async function waitForAnimations(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );
}
