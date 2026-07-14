import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/signout";

test.describe("signout screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });
  });

  test("signout-section — Sign Out card in Settings › Profile", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "profile");

    const section = page.getByTestId("settings-signout");
    await section.scrollIntoViewIfNeeded();

    // Settle animations before capture.
    await page.evaluate(() =>
      Promise.all(document.getAnimations().map((a) => a.finished)),
    );
    await page.waitForTimeout(200);

    await section.screenshot({ path: `${SHOTS}/signout-section.png` });
  });

  test("signout-dialog — AlertDialog shown before the wipe runs", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "profile");

    const section = page.getByTestId("settings-signout");
    await section.scrollIntoViewIfNeeded();

    // Open the confirmation dialog.
    await page.getByTestId("signout-open-dialog").click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await expect(dialog.getByText("Sign out and wipe all data?")).toBeVisible();

    // Settle animations before capture.
    await page.evaluate(() =>
      Promise.all(document.getAnimations().map((a) => a.finished)),
    );
    await page.waitForTimeout(200);

    await page.screenshot({ path: `${SHOTS}/signout-dialog.png` });
  });
});
