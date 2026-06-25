import { expect, test } from "@playwright/test";

import { installMockBridge, openChannelBrowser } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("keyboard shortcut opens the channel browser dialog", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  const isMacBrowser = await page.evaluate(() =>
    /mac|iphone|ipad|ipod/i.test(navigator.platform),
  );

  if (isMacBrowser) {
    await page.evaluate(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          key: "O",
          metaKey: true,
          shiftKey: true,
        }),
      );
    });
  } else {
    await page.keyboard.press("Control+Shift+O");
  }
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
});

test("channel browser shows channels not yet joined", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // "design" and "sales" are open channels the mock user is NOT a member of
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();
  await expect(page.getByTestId("browse-channel-sales")).toBeVisible();

  // "general" is a channel the mock user IS a member of — shown in "Joined" section
  await expect(page.getByTestId("browse-channel-general")).toBeVisible();
});

test("channel browser search filters by name", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  await page.getByTestId("channel-browser-search").fill("design");

  await expect(page.getByTestId("browse-channel-design")).toBeVisible();
  await expect(page.getByTestId("browse-channel-sales")).toHaveCount(0);
  await expect(page.getByTestId("browse-channel-general")).toHaveCount(0);
});

test("channel browser search filters by description", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("pipeline");

  // "sales" has "pipeline" in its description
  await expect(page.getByTestId("browse-channel-sales")).toBeVisible();
  await expect(page.getByTestId("browse-channel-design")).toHaveCount(0);
});

test("channel browser shows no results for unmatched search", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("zzz-nonexistent");

  await expect(page.getByText("No channels match your search")).toBeVisible();
});

test("joining a channel from browser adds it to the sidebar", async ({
  page,
}) => {
  await page.goto("/");

  // Verify "design" is not in the sidebar
  const streamList = page.getByTestId("stream-list");
  await expect(streamList).not.toContainText("design");

  // Open browser and join
  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-design")
    .getByRole("button", { name: "Join" })
    .click();

  // Dialog should close and navigate to the joined channel
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page).toHaveURL(/#\/channels\//);
  await expect(page.getByTestId("chat-title")).toHaveText("design");

  // Channel should now appear in the sidebar
  await expect(streamList).toContainText("design");
});

test("clicking a joined channel in browser navigates to it", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // "general" is already joined — clicking should navigate without join
  await page.getByTestId("browse-channel-general").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page).toHaveURL(/#\/channels\//);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("channel browser does not show DM or private channels", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // DM channels should not appear
  await expect(page.getByTestId("browse-channel-alice-tyler")).toHaveCount(0);
  await expect(page.getByTestId("browse-channel-bob-tyler")).toHaveCount(0);

  // Private forum should not appear
  await expect(page.getByTestId("browse-channel-announcements")).toHaveCount(0);
});

test("channel browser closes on escape", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
});

test("keyboard navigation works in channel browser", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // Filter to unjoined channels only to get a predictable list
  await page.getByTestId("channel-browser-search").fill("design");
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();

  // Press Enter to join the selected (first) channel
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText("design");
});

test("sidebar only shows channels the user has joined", async ({ page }) => {
  await page.goto("/");

  const streamList = page.getByTestId("stream-list");

  // Channels the mock user IS a member of
  await expect(streamList).toContainText("general");
  await expect(streamList).toContainText("random");
  await expect(streamList).toContainText("engineering");
  await expect(streamList).toContainText("agents");

  // Channels the mock user is NOT a member of
  await expect(streamList).not.toContainText("design");
  await expect(streamList).not.toContainText("sales");
});
