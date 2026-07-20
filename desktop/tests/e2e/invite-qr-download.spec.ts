import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
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

test("invite QR reuses the media menu and saves its PNG", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, "community-members");
  await expect(page.getByTestId("settings-community-members")).toBeVisible();

  await page.getByTestId("create-invite-link").click();
  const qrCode = page.getByTestId("invite-link-qr-code");
  await expect(qrCode).toBeVisible();

  await qrCode.click({ button: "right", position: { x: 32, y: 32 } });
  const menu = page.locator("[data-invite-qr-context-menu]");
  await expect(menu).toBeVisible();
  await menu.getByRole("button", { name: "Download image" }).click();
  await expect(menu).not.toBeVisible();

  const payload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: Record<string, unknown> | null;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.find(({ command }) => command === "save_png_data_url")?.payload;
  });

  expect(payload?.filename).toBe("buzz-community-invite.png");
  expect(payload?.dataUrl).toMatch(/^data:image\/png;base64,/);
});
