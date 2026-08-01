import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("opens the signed channel panel and preserves its empty state", async ({
  page,
}) => {
  await page.getByTestId("channel-panel-trigger").click();

  await expect(page.getByTestId("signed-channel-panel")).toBeVisible();
  await expect(page.getByTestId("signed-channel-panel-empty")).toBeVisible();
  await expect(page.getByTestId("signed-channel-panel-empty")).toContainText(
    "no signed panel projection",
  );
  await expect(page).toHaveURL(/panel=%221%22/);

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("signed-channel-panel")).toBeHidden();
  await expect(page).not.toHaveURL(/panel=%22/);
});
