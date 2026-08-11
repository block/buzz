/**
 * Screenshot spec for Buzz Term visible empty/error states (issue #4930).
 *
 * The mock bridge does not implement `terminal_attach`, so opening the Term
 * panel in the e2e build exercises the real attach-failure path end to end:
 * the panel must render a visible "Terminal unavailable" notice with a Retry
 * action instead of the silent gray body shipped today.
 */

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/term-empty-states";

test.describe("terminal empty/error states", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test.beforeEach(async ({ page }) => {
    // windowLabel makes the bridge set window.isTauri, so the panel takes the
    // real attach path; the bridge then rejects the unmocked terminal_attach,
    // exercising the attach-failure notice end to end.
    await installMockBridge(page, { windowLabel: "main" });
  });

  test("attach failure renders a visible notice with retry", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("message-composer")).toBeVisible({
      timeout: 10_000,
    });

    await page.keyboard.press("Control+j");

    const notice = page.getByTestId("terminal-notice");
    await expect(notice).toBeVisible({ timeout: 10_000 });
    await expect(notice).toContainText("Terminal unavailable");
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

    // The unread pill animates in over the sidebar and would otherwise land in
    // the shot at a nondeterministic opacity.
    await page
      .getByText("1 new message")
      .waitFor({ state: "hidden", timeout: 15_000 })
      .catch(() => {});
    await waitForAnimations(page);
    await page
      .locator(".buzz-terminal-substrate")
      .screenshot({ path: `${SHOTS}/01-term-attach-error.png` });
  });
});
