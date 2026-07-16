import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

type E2eWindow = Window & {
  __BUZZ_E2E_COMMANDS__?: string[];
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("Share compute has a clear empty state and starts and stops sharing", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");
  const model = page.getByTestId("mesh-share-compute-model");

  await expect(card).toContainText("Not sharing right now");
  await expect(card).toContainText(
    "Choose a suggested model below, or enter a model reference or local file",
  );
  await expect(toggle).toBeDisabled();

  await model.fill("hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M");
  await expect(card).toContainText(
    "Buzz downloads remote models when sharing starts",
  );
  await expect(toggle).toBeEnabled();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(card).toContainText("Sharing SmolLM2 135M with relay members");
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_start_node");

  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(card).toContainText("Not sharing right now");
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_stop_node");
});
