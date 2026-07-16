import { test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { openSettings } from "../helpers/settings";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("share-compute recommended + advanced shared models", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  await card.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/screenshots/01-share-compute-recommended.png",
  });

  // Expand Advanced to reveal the "Join a shared model" section.
  await card.getByText("Advanced").click();
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/screenshots/02-share-compute-advanced-shared.png",
  });
});
