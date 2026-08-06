import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import { waitForAnimations } from "../helpers/animations";

test("captures the Community Compute tutorial", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "compute");
  await page.getByTestId("community-compute-tutorial-button").click();
  await expect(
    page.getByTestId("community-compute-territory-map"),
  ).toBeVisible();
  await expect(page.getByTestId("compute-share-banner")).toHaveCount(0);
  await waitForAnimations(page);
  await page.getByTestId("settings-panel-compute").screenshot({
    path: "test-results/mesh-compute/community-tutorial.png",
  });
  await page.getByTestId("community-compute-territory-map").screenshot({
    path: "test-results/mesh-compute/community-map.png",
  });

  await page.getByLabel("Preview scenario").selectOption("large-dense-500");
  await expect(
    page.getByTestId("community-compute-kpi-members-sharing"),
  ).toContainText("458");
  await expect(page.locator("[data-cell-count]")).toHaveCount(458);
  await expect(
    page.locator('[data-inference-active="true"]').first(),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("settings-panel-compute").screenshot({
    path: "test-results/mesh-compute/community-dense.png",
  });
});
