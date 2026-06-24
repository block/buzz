import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/home-collapsed-top-chrome";

test.describe("home inbox header collapsed-sidebar chrome clearance", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("inbox options clear the macOS traffic-light region when sidebar is collapsed", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    await page.locator('[data-sidebar="trigger"]').click();

    const inboxOptions = page.getByTestId("inbox-options-trigger");
    await expect(inboxOptions).toBeVisible();
    await expect
      .poll(async () =>
        inboxOptions.evaluate((element) =>
          Math.round(element.getBoundingClientRect().left),
        ),
      )
      .toBeGreaterThanOrEqual(168);

    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/01-collapsed-inbox-header-clears-traffic-lights.png`,
      clip: { x: 0, y: 0, width: 420, height: 120 },
    });
  });
});
