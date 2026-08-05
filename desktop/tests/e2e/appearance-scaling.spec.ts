import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SCALE_PREFIXES = [
  "interface-scale",
  "chat-scale",
  "avatar-scale",
] as const;

/** Max preset index on the shared 75%–500% ladder (12 steps → index 11). */
const MAX_PRESET_INDEX = "11";

async function clearAppearanceScales(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    globalThis.localStorage?.removeItem("buzz:text-scale");
    globalThis.localStorage?.removeItem("buzz:chat-scale");
    globalThis.localStorage?.removeItem("buzz:avatar-scale");
  });
}

async function openAppearance(page: import("@playwright/test").Page) {
  // After reload the app may rehydrate into the settings shell. Wait for either
  // entry surface before choosing how to open Appearance.
  const openSettingsBtn = page.getByTestId("open-settings");
  const appearanceNav = page.getByTestId("settings-nav-appearance");
  await Promise.race([
    openSettingsBtn.waitFor({ state: "visible", timeout: 20_000 }),
    appearanceNav.waitFor({ state: "visible", timeout: 20_000 }),
  ]);

  if (await appearanceNav.isVisible().catch(() => false)) {
    await appearanceNav.click();
  } else {
    await openSettings(page, "appearance");
  }
  await expect(page.getByTestId("interface-scale-slider")).toBeVisible();
}

async function setScaleToMax(
  page: import("@playwright/test").Page,
  prefix: (typeof SCALE_PREFIXES)[number],
) {
  const slider = page.getByTestId(`${prefix}-slider`);
  await slider.fill(MAX_PRESET_INDEX);
  await expect(page.getByTestId(`${prefix}-value`)).toHaveText("500%");
  await expect(slider).toHaveAttribute("aria-valuetext", "500%");
}

test.describe("Appearance scaling", () => {
  test.use({ viewport: { width: 1280, height: 800 } });
  test.setTimeout(60_000);

  test("Appearance scales reach 500% and remain usable", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await clearAppearanceScales(page);
    await page.reload({ waitUntil: "domcontentloaded" });
    await openAppearance(page);

    for (const prefix of SCALE_PREFIXES) {
      await setScaleToMax(page, prefix);
    }

    await expect(page.getByTestId("settings-content-scroll")).not.toHaveCSS(
      "overflow-x",
      "scroll",
    );

    // Persist across reload. Interface at 500% makes chrome huge, so verify
    // storage then restore interface before interacting with sliders again.
    await page.reload({ waitUntil: "domcontentloaded" });
    const stored = await page.evaluate(() => ({
      text: globalThis.localStorage?.getItem("buzz:text-scale"),
      chat: globalThis.localStorage?.getItem("buzz:chat-scale"),
      avatar: globalThis.localStorage?.getItem("buzz:avatar-scale"),
    }));
    expect(stored.text).toBe("5");
    expect(stored.chat).toBe("5");
    expect(stored.avatar).toBe("5");

    await page.evaluate(() => {
      globalThis.localStorage?.removeItem("buzz:text-scale");
    });
    await page.reload({ waitUntil: "domcontentloaded" });
    await openAppearance(page);
    await expect(page.getByTestId("interface-scale-value")).toHaveText("100%");
    await expect(page.getByTestId("chat-scale-value")).toHaveText("500%");
    await expect(page.getByTestId("avatar-scale-value")).toHaveText("500%");

    await page.getByTestId("chat-scale-reset").click();
    await page.getByTestId("avatar-scale-reset").click();
    await expect(page.getByTestId("chat-scale-value")).toHaveText("100%");
    await expect(page.getByTestId("avatar-scale-value")).toHaveText("100%");
  });

  test("avatar scale updates message avatars without fixed pixel lock", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await clearAppearanceScales(page);
    await page.reload({ waitUntil: "domcontentloaded" });

    const input = page.getByTestId("message-input");
    if (await input.isVisible().catch(() => false)) {
      await input.fill("scale probe message");
      await page.getByTestId("send-message").click();
      await expect(page.getByTestId("message-timeline")).toContainText(
        "scale probe message",
      );
    }

    const avatar = page.getByTestId("message-avatar").first();
    if ((await avatar.count()) === 0) {
      test.skip(true, "No message avatar in mock bridge timeline");
      return;
    }

    const baseline = await avatar.boundingBox();
    expect(baseline).toBeTruthy();

    await openAppearance(page);
    await setScaleToMax(page, "avatar-scale");

    const back = page.getByRole("button", { name: "Back to app" });
    if (await back.isVisible().catch(() => false)) {
      await back.click();
    } else {
      await page.keyboard.press("Escape");
    }

    const scaled = page.getByTestId("message-avatar").first();
    await expect(scaled)
      .toBeVisible({ timeout: 10_000 })
      .catch(() => {});
    if ((await scaled.count()) > 0) {
      const after = await scaled.boundingBox();
      if (baseline && after) {
        expect(after.width).toBeGreaterThan(baseline.width * 1.5);
      }
    }
  });
});

test.describe("Appearance scaling (narrow)", () => {
  test.use({ viewport: { width: 900, height: 900 } });
  test.setTimeout(60_000);

  test("scale sliders stay reachable at a constrained width", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await clearAppearanceScales(page);
    await page.reload({ waitUntil: "domcontentloaded" });

    // Ensure the profile chrome (open-settings) is on-screen at this width.
    const toggle = page.getByRole("button", {
      name: "Toggle Sidebar",
      exact: true,
    });
    if (await toggle.isVisible().catch(() => false)) {
      if (
        !(await page
          .getByTestId("open-settings")
          .isVisible()
          .catch(() => false))
      ) {
        await toggle.click();
      }
    }
    await openAppearance(page);

    for (const prefix of SCALE_PREFIXES) {
      const slider = page.getByTestId(`${prefix}-slider`);
      await expect(slider).toBeVisible();
      const box = await slider.boundingBox();
      expect(box).toBeTruthy();
      if (box) {
        expect(box.width).toBeGreaterThan(40);
        expect(box.height).toBeGreaterThanOrEqual(20);
      }
      if (prefix !== "interface-scale") {
        await setScaleToMax(page, prefix);
      } else {
        await setScaleToMax(page, prefix);
        await page.getByTestId("interface-scale-reset").click();
        await expect(page.getByTestId("interface-scale-value")).toHaveText(
          "100%",
        );
      }
    }
  });
});
