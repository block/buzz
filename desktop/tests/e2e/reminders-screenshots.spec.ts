import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/reminders";

test.describe("reminders screenshots", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("01 — sidebar shows Reminders nav item", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const remindersNav = page.getByTestId("open-reminders-view");
    await expect(remindersNav).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/01-sidebar-reminders-nav.png`,
      clip: { x: 0, y: 0, width: 256, height: 720 },
    });
  });

  test("02 — message action menu shows Remind me later", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const messageRow = page.getByTestId("message-row").first();
    await messageRow.hover();

    const moreActionsButton = messageRow.getByRole("button", {
      name: "More actions",
    });
    await expect(moreActionsButton).toBeVisible();
    await moreActionsButton.click();

    const remindItem = page.getByRole("menuitem", {
      name: "Remind me later",
    });
    await expect(remindItem).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/02-message-action-remind-later.png`,
    });
  });

  test("03 — Remind me later dialog", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const messageRow = page.getByTestId("message-row").first();
    await messageRow.hover();

    const moreActionsButton = messageRow.getByRole("button", {
      name: "More actions",
    });
    await moreActionsButton.click();

    const remindItem = page.getByRole("menuitem", {
      name: "Remind me later",
    });
    await remindItem.click();

    // Wait for the dialog to appear
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Remind me later")).toBeVisible();
    await expect(dialog.getByText("In 30 minutes")).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/03-remind-me-later-dialog.png`,
    });
  });

  test("04 — Reminders panel empty state", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Navigate to reminders via sidebar button
    await page.getByTestId("open-reminders-view").click();

    // Wait for the panel to render (empty state)
    await expect(page.getByText("No pending reminders")).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/04-reminders-panel-empty.png`,
    });
  });
});
