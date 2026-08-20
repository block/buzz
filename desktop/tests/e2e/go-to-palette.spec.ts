import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const primary = process.platform === "darwin" ? "Meta" : "Control";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

// The ⌘G listener mounts with the app shell; wait for it before pressing keys.
async function gotoReady(page: import("@playwright/test").Page) {
  await page.goto("/");
  await expect(page.getByTestId("sidebar-primary-menu")).toBeVisible();
}

test("⌘G opens the Go to palette and ⌘+letter jumps by mnemonic", async ({
  page,
}) => {
  await gotoReady(page);

  await page.keyboard.press(`${primary}+g`);

  await expect(page.getByTestId("go-to-palette")).toBeVisible();
  await expect(page.getByTestId("go-to-input")).toBeFocused();

  // Cmd/Ctrl+A is the stable global mnemonic for Agents.
  await page.keyboard.press(`${primary}+a`);

  await expect(page).toHaveURL(/#\/agents$/);
  await expect(page.getByTestId("agents-page-content")).toBeVisible();
  await expect(page.getByTestId("go-to-palette")).not.toBeVisible();
});

test("a bare digit jumps by visible row position", async ({ page }) => {
  await gotoReady(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page).toHaveURL(/#\/agents$/);

  await page.keyboard.press(`${primary}+g`);
  await expect(page.getByTestId("go-to-palette")).toBeVisible();

  // Row 1 is Inbox; the bare digit accelerator must not type into the filter.
  await page.keyboard.press("1");

  await expect(page).toHaveURL(/#\/$/);
  await expect(page.getByTestId("go-to-input")).toHaveCount(0);
});

test("type-to-filter narrows the list and Enter selects the row", async ({
  page,
}) => {
  await gotoReady(page);

  await page.keyboard.press(`${primary}+g`);
  await expect(page.getByTestId("go-to-palette")).toBeVisible();

  await page.getByTestId("go-to-input").fill("age");
  await expect(page.getByTestId("go-to-item-agents")).toBeVisible();
  await expect(page.getByTestId("go-to-item-inbox")).toHaveCount(0);

  await page.keyboard.press("Enter");

  await expect(page).toHaveURL(/#\/agents$/);
  await expect(page.getByTestId("agents-page-content")).toBeVisible();
});

test("Escape closes the palette without navigating", async ({ page }) => {
  await gotoReady(page);
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const channelUrl = page.url();

  await page.keyboard.press(`${primary}+g`);
  await expect(page.getByTestId("go-to-palette")).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.getByTestId("go-to-palette")).not.toBeVisible();
  await expect.poll(() => page.url()).toBe(channelUrl);
});
