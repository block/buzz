import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// The macOS traffic lights are native chrome: with `trafficLightPosition`
// x:16 they occupy roughly x 16–68 regardless of the app's Cmd +/- text
// zoom. The top-chrome nav row must clear that band in fixed px, so the
// clearance cannot shrink when the root font size scales down.
const TRAFFIC_LIGHT_RIGHT_EDGE = 72;

async function spoofMacPlatform(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
  });
}

async function firstNavButtonX(page: import("@playwright/test").Page) {
  const toggle = page.locator('[data-testid="app-top-chrome"] button').first();
  await expect(toggle).toBeVisible();
  const box = await toggle.boundingBox();
  expect(box).not.toBeNull();
  return box?.x ?? 0;
}

// The chrome buttons are styled to visually match the fixed-size native
// controls, so their box must not follow the rem text scale either. The
// sidebar toggle is 28px square; the back/forward history buttons share the
// height but are deliberately narrower (24px).
const NAV_BUTTON_SIZE = 28;
const HISTORY_BUTTON_WIDTH = 24;

// The grabber/drag strip hosting the buttons must hold its height too —
// otherwise Cmd+ balloons the bar around the fixed-size buttons and Cmd-
// collapses it.
const TOP_CHROME_BAR_HEIGHT = 40;

async function expectTopChromeFixedHeight(
  page: import("@playwright/test").Page,
) {
  const bar = page.getByTestId("app-top-chrome");
  await expect(bar).toBeVisible();
  const box = await bar.boundingBox();
  expect(box).not.toBeNull();
  expect(box?.height ?? 0).toBe(TOP_CHROME_BAR_HEIGHT);
}

async function expectNavButtonsFixedSize(
  page: import("@playwright/test").Page,
) {
  const buttons = page.locator('[data-testid="app-top-chrome"] button');
  const count = await buttons.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    const button = buttons.nth(i);
    const label = await button.getAttribute("aria-label");
    const isHistoryButton = label === "Go back" || label === "Go forward";
    const box = await button.boundingBox();
    expect(box).not.toBeNull();
    expect(box?.width ?? 0).toBe(
      isHistoryButton ? HISTORY_BUTTON_WIDTH : NAV_BUTTON_SIZE,
    );
    expect(box?.height ?? 0).toBe(NAV_BUTTON_SIZE);
  }
}

async function seedTextScale(
  page: import("@playwright/test").Page,
  scale: number,
) {
  await page.addInitScript((value) => {
    window.localStorage.setItem("buzz:text-scale", String(value));
  }, scale);
}

async function expectRootFontSize(
  page: import("@playwright/test").Page,
  fontSize: string,
) {
  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.documentElement).fontSize),
    )
    .toBe(fontSize);
}

test.describe("top chrome macOS traffic-light clearance under text zoom", () => {
  test("nav buttons clear the traffic lights at default zoom", async ({
    page,
  }) => {
    await spoofMacPlatform(page);
    await installMockBridge(page);
    await page.goto("/");

    expect(await firstNavButtonX(page)).toBeGreaterThanOrEqual(
      TRAFFIC_LIGHT_RIGHT_EDGE,
    );
    await expectNavButtonsFixedSize(page);
    await expectTopChromeFixedHeight(page);
  });

  test("nav buttons still clear the traffic lights when zoomed out", async ({
    page,
  }) => {
    await spoofMacPlatform(page);
    // Seed the minimum Cmd- text scale (0.75). The old rem-based clearance
    // (pl-20 = 5rem) shrank to 60px here, sliding the buttons under the
    // fixed-position native controls.
    await seedTextScale(page, 0.75);
    await installMockBridge(page);
    await page.goto("/");

    // Confirm the zoomed-out scale actually applied to the root font size.
    await expectRootFontSize(page, "12px");

    expect(await firstNavButtonX(page)).toBeGreaterThanOrEqual(
      TRAFFIC_LIGHT_RIGHT_EDGE,
    );
    await expectNavButtonsFixedSize(page);
    await expectTopChromeFixedHeight(page);
  });

  test("nav buttons keep their fixed size when zoomed in", async ({ page }) => {
    await spoofMacPlatform(page);
    // Seed the maximum Cmd+ text scale (1.5). Rem-sized buttons (h-7 = 42px
    // here) grew visibly taller than the fixed-size traffic lights.
    await seedTextScale(page, 1.5);
    await installMockBridge(page);
    await page.goto("/");

    await expectRootFontSize(page, "24px");

    expect(await firstNavButtonX(page)).toBeGreaterThanOrEqual(
      TRAFFIC_LIGHT_RIGHT_EDGE,
    );
    await expectNavButtonsFixedSize(page);
    await expectTopChromeFixedHeight(page);
  });
});
