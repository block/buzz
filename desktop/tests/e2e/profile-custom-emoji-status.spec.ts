import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHORTCODE = "sprout";
const STATUS_TEXT = "testing custom status";

async function openProfilePopover(page: import("@playwright/test").Page) {
  await page.getByTestId("open-settings").click();
  await expect(page.getByTestId("profile-popover")).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  const PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR4nGMwuBPxnxLMMGrAqAGjBgwXAwBwOGMf1PPhVwAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({ contentType: "image/png", body: PNG }),
  );
});

test("profile popover renders a custom emoji status as an image", async ({
  page,
}) => {
  await page.goto("/");
  await openProfilePopover(page);

  await page.getByTestId("profile-popover-set-status").click();
  await expect(page.getByTestId("set-status-dialog")).toBeVisible();
  await page.getByLabel("Choose status emoji").click();

  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(SHORTCODE);
  await picker
    .getByRole("button", { name: `:${SHORTCODE}:` })
    .first()
    .click();
  await page.getByTestId("set-status-input").fill(STATUS_TEXT);
  await page.getByTestId("set-status-save").click();

  await openProfilePopover(page);

  const statusButton = page.getByTestId("profile-popover-set-status");
  await expect(statusButton).toContainText(STATUS_TEXT);
  await expect(statusButton.locator(`img[alt=":${SHORTCODE}:"]`)).toBeVisible();
  await expect(statusButton).not.toContainText(`:${SHORTCODE}:`);
});
