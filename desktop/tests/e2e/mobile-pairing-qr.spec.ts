import { expect, test } from "@playwright/test";
import { mkdirSync } from "node:fs";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SCREENSHOT_DIR = "test-results/mobile-pairing-qr";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("mobile pairing uses the local Wallet-style QR renderer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-mobile").click();
  await page.getByTestId("pair-mobile-button").click();

  const dialog = page.getByTestId("mobile-pairing-dialog");
  const qrCode = page.getByTestId("mobile-pairing-qr");
  const qrContainer = page.getByTestId("mobile-pairing-qr-container");
  const copyButton = dialog.getByTestId("copy-pairing-code");
  await expect(dialog).toBeVisible();
  await expect(qrCode).toBeVisible();
  await expect(copyButton).toHaveText("Copy pairing code");
  await expect(dialog).toHaveCSS("border-radius", "16px");
  await expect(dialog).toHaveCSS("border-left-width", "0px");
  await expect(dialog).toHaveCSS("padding-left", "24px");
  await expect(dialog).toHaveCSS("padding-right", "24px");
  await expect(dialog).toHaveCSS("padding-top", "24px");
  await expect(dialog.getByTestId("mobile-pairing-done")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Close", exact: true }),
  ).toHaveCount(1);
  await expect(
    dialog.getByText("Waiting for mobile device to scan..."),
  ).toHaveCount(0);
  await expect(dialog.locator("code")).toHaveCount(0);
  await expect(qrCode).toHaveAttribute("data-qr-matrix-size", "57");
  await expect(qrCode.locator("[data-qr-finder-pattern]")).toHaveCount(3);
  expect(await qrCode.locator("circle").count()).toBeGreaterThan(100);
  await expect(qrCode.locator("image")).toHaveAttribute(
    "href",
    "/app-icon@2x.png",
  );
  await waitForAnimations(page);
  const qrContainerWidth = await qrContainer.evaluate(
    (element) => getComputedStyle(element).width,
  );
  await expect(copyButton).toHaveCSS("width", qrContainerWidth);

  mkdirSync(SCREENSHOT_DIR, { recursive: true });
  await dialog.screenshot({ path: `${SCREENSHOT_DIR}/pairing-dialog.png` });
  await qrCode.screenshot({ path: `${SCREENSHOT_DIR}/pairing-qr.png` });
});
