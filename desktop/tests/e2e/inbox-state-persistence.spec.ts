import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const MOCK_RELAY_ENCODED = encodeURIComponent("ws://localhost:3000");
const VIEW_STORAGE_KEY = `buzz-inbox-view.v1:${MOCK_PUBKEY}:${MOCK_RELAY_ENCODED}`;

async function expectStickyInboxState(page: Page) {
  await expect(page.getByTestId("inbox-filter-trigger")).toHaveText("Mentions");
  await page.getByTestId("inbox-options-trigger").click();
  await expect(page.getByTestId("inbox-unread-only-toggle")).toBeChecked();
}

test("Inbox filter and unread-only survive navigation and reload", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Mentions" }).click();
  await page.getByTestId("inbox-options-trigger").click();
  await page.getByTestId("inbox-unread-only-toggle").click();
  await expect(page.getByTestId("inbox-unread-only-toggle")).toBeChecked();
  await page.keyboard.press("Escape");

  await expect
    .poll(() =>
      page.evaluate((key) => {
        return JSON.parse(window.localStorage.getItem(key) ?? "null");
      }, VIEW_STORAGE_KEY),
    )
    .toEqual({ version: 1, filter: "mention", unreadOnly: true });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await expectStickyInboxState(page);
  await page.keyboard.press("Escape");

  await page.reload();
  await expectStickyInboxState(page);
});
