import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/history-icons";

test.describe("top chrome history controls", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("back and forward buttons read as a compact pair", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    const back = page.getByTestId("global-back");
    const forward = page.getByTestId("global-forward");
    await expect(back).toBeVisible();
    await expect(forward).toBeVisible();

    await expect
      .poll(async () =>
        page.evaluate(() => {
          const backRect = document
            .querySelector('[data-testid="global-back"]')
            ?.getBoundingClientRect();
          const forwardRect = document
            .querySelector('[data-testid="global-forward"]')
            ?.getBoundingClientRect();
          if (!backRect || !forwardRect) return null;

          const backCenter = backRect.left + backRect.width / 2;
          const forwardCenter = forwardRect.left + forwardRect.width / 2;
          return Math.round(forwardCenter - backCenter);
        }),
      )
      .toBeLessThanOrEqual(28);

    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/01-history-icons-compact-unit.png`,
      clip: { x: 64, y: 0, width: 130, height: 48 },
    });
  });
});
