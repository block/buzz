import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test.beforeEach(async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });
  await installMockBridge(page, {
    relayRequiresMembership: true,
  });
  await page.route("**/api/invites", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      json: {
        code: "qr-download-test",
        expires_at: Math.floor(Date.now() / 1000) + 86_400,
        url: "buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=qr-download-test",
      },
      status: 200,
    });
  });
});

test("copies a freshly minted invite link without showing a URL or QR code", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, "community-members");
  await expect(page.getByTestId("settings-community-members")).toBeVisible();

  await page.getByTestId("community-invite-dialog-trigger").click();
  await expect(page.getByTestId("invite-link-url")).toHaveCount(0);
  await expect(page.getByTestId("invite-link-qr-code")).toHaveCount(0);
  await page.getByTestId("copy-invite-link").click();
  await expect(page.getByTestId("copy-invite-link")).toContainText("Copied");

  const payload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: Record<string, unknown> | null;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.findLast(({ command }) => command === "copy_text_to_clipboard")
      ?.payload;
  });

  expect(payload).toEqual({
    text: "buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=qr-download-test",
  });
});
