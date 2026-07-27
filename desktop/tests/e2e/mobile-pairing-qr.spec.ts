import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SCREENSHOT_DIR = "test-results/mobile-pairing-qr";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, { pairingStartDelayMs: 300 });
});

async function emitPairingEvent(page: Page, event: string, payload?: unknown) {
  await page.evaluate(
    async ({ eventName, eventPayload }) => {
      const internals = (
        window as Window & {
          __TAURI_INTERNALS__?: {
            invoke?: (
              command: string,
              args: Record<string, unknown>,
            ) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__;
      if (!internals?.invoke) {
        throw new Error("Tauri E2E event bridge is unavailable");
      }
      await internals.invoke("plugin:event|emit", {
        event: eventName,
        payload: eventPayload,
      });
    },
    { eventName: event, eventPayload: payload },
  );
}

test("mobile pairing starts on demand and reveals the QR code", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-mobile").click();

  mkdirSync(SCREENSHOT_DIR, { recursive: true });

  const section = page.getByTestId("settings-mobile");
  const card = page.getByTestId("mobile-pairing-card");
  const qrContainer = page.getByTestId("mobile-pairing-qr-container");
  const startButton = card.getByTestId("start-pairing-button");
  await expect(card).toBeVisible();
  await expect(startButton).toHaveText("Start pairing");
  await expect(page.getByTestId("mobile-pairing-qr")).toHaveCount(0);
  await expect(page.getByTestId("copy-pairing-code")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "start_pairing",
        ).length,
    ),
  ).toBe(0);

  const sectionBox = await section.boundingBox();
  const cardBox = await card.boundingBox();
  expect(sectionBox).not.toBeNull();
  expect(cardBox).not.toBeNull();
  const sectionCenter = (sectionBox?.x ?? 0) + (sectionBox?.width ?? 0) / 2;
  const cardCenter = (cardBox?.x ?? 0) + (cardBox?.width ?? 0) / 2;
  expect(Math.abs(sectionCenter - cardCenter)).toBeLessThan(0.5);
  await waitForAnimations(page);
  await card.screenshot({ path: `${SCREENSHOT_DIR}/pairing-start.png` });

  await startButton.click();

  const loadingSpinner = card.getByTestId("pairing-loading-spinner");
  await expect(loadingSpinner).toBeVisible();
  await expect(loadingSpinner).toHaveCSS("animation-name", "spin");

  const qrCode = page.getByTestId("mobile-pairing-qr");
  const copyButton = card.getByTestId("copy-pairing-code");
  await expect(qrCode).toBeVisible();
  await expect(copyButton).toHaveText("Copy pairing code");
  await expect(startButton).toHaveCount(0);
  await expect(card.getByText("Pair mobile device")).toHaveCount(0);
  await expect(page.getByTestId("mobile-pairing-dialog")).toHaveCount(0);
  await expect(
    page.getByText("Securely transfer your identity via NIP-AB protocol"),
  ).toHaveCount(0);
  await expect(qrCode).toHaveAttribute("data-qr-matrix-size", "57");
  await expect(qrCode.locator("[data-qr-finder-pattern]")).toHaveCount(3);
  expect(await qrCode.locator("circle").count()).toBeGreaterThan(100);
  expect(await qrCode.locator(".buzz-qr-cell-reveal").count()).toBeGreaterThan(
    100,
  );
  await expect(qrCode.locator(".buzz-qr-cell-reveal").first()).toHaveCSS(
    "animation-name",
    "buzz-qr-cell-reveal",
  );
  await expect(qrCode.locator(".buzz-qr-cell-reveal").first()).toHaveCSS(
    "animation-duration",
    "0.058s",
  );
  await expect(qrCode.locator(".buzz-qr-cell-reveal").first()).toHaveCSS(
    "animation-timing-function",
    "linear",
  );
  await expect(
    qrCode.locator('[data-qr-cell-row="56"].buzz-qr-cell-reveal').first(),
  ).toHaveCSS("animation-delay", "0.189s");
  await expect(copyButton).toHaveCSS("animation-name", "enter");
  await expect(copyButton).toHaveCSS("animation-duration", "0.25s");
  await expect(qrCode.locator("image")).toHaveAttribute(
    "href",
    "/app-icon@2x.png",
  );
  await waitForAnimations(page);
  const qrBox = await qrContainer.boundingBox();
  const copyBox = await copyButton.boundingBox();
  expect(qrBox).not.toBeNull();
  expect(copyBox).not.toBeNull();
  expect(copyBox?.y ?? 0).toBeGreaterThan(
    (qrBox?.y ?? 0) + (qrBox?.height ?? 0),
  );
  expect(Math.abs((copyBox?.x ?? 0) - (qrBox?.x ?? 0))).toBeLessThan(0.5);
  expect(Math.abs((copyBox?.width ?? 0) - (qrBox?.width ?? 0))).toBeLessThan(
    0.5,
  );

  await emitPairingEvent(page, "pairing-error", {
    message: "Session timed out",
  });

  await expect(qrCode).toHaveCount(0);
  await expect(copyButton).toHaveCount(0);
  await expect(card.getByText("Pairing code expired.")).toBeVisible();

  const regenerateButton = card.getByTestId("regenerate-pairing-button");
  await expect(regenerateButton).toHaveText("Generate new pairing code");
  await waitForAnimations(page);
  await card.screenshot({ path: `${SCREENSHOT_DIR}/pairing-expired.png` });
  await regenerateButton.click();
  await expect(loadingSpinner).toBeVisible();
  await expect(qrCode).toBeVisible();
  await expect(copyButton).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "start_pairing",
        ).length,
    ),
  ).toBe(2);

  await waitForAnimations(page);
  await card.screenshot({ path: `${SCREENSHOT_DIR}/pairing-card.png` });
  await qrCode.screenshot({ path: `${SCREENSHOT_DIR}/pairing-qr.png` });
});

test("late pairing events are ignored after canceling", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-mobile").click();

  const card = page.getByTestId("mobile-pairing-card");
  await card.getByTestId("start-pairing-button").click();
  await expect(page.getByTestId("mobile-pairing-qr")).toBeVisible();

  await emitPairingEvent(page, "pairing-sas-received", { sas: "123456" });
  const dialog = page.getByTestId("mobile-pairing-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Close" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(card.getByText("Pairing was canceled.")).toBeVisible();

  await emitPairingEvent(page, "pairing-complete");
  await emitPairingEvent(page, "pairing-sas-received", { sas: "654321" });

  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("mobile-pairing-done")).toHaveCount(0);
  await expect(card.getByText("Pairing was canceled.")).toBeVisible();
});
