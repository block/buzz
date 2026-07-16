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

  await expect(card).toContainText("Not sharing right now");
  await expect(card).toContainText("Recommended model");
  await expect(toggle).toBeDisabled();

  // The free-text model field lives under Advanced now.
  await card.getByText("Advanced").click();
  const model = page.getByTestId("mesh-share-compute-model");
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

test("Advanced offers shared models and shows a waiting-for-members state", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");

  await card.getByText("Advanced").click();

  // The shared-model picker lists layer packages with a member estimate and an
  // honest slower-than-solo warning.
  const sharedPicker = page.getByTestId("mesh-shared-model-picker");
  await expect(sharedPicker).toContainText("Join a shared model");
  await expect(sharedPicker).toContainText("the group trades speed for size");
  await expect(sharedPicker).toContainText("~7 members");

  // Selecting a shared model and starting shows the cohort waiting state
  // instead of a plain solo "Starting…".
  await page
    .getByTestId("mesh-shared-meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers")
    .click();
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(card).toContainText(
    "Waiting for more members to join before this shared model can run",
  );
});
