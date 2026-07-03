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
  });

  test("nav buttons still clear the traffic lights when zoomed out", async ({
    page,
  }) => {
    await spoofMacPlatform(page);
    // Seed the minimum Cmd- text scale (0.75). The old rem-based clearance
    // (pl-20 = 5rem) shrank to 60px here, sliding the buttons under the
    // fixed-position native controls.
    await page.addInitScript(() => {
      window.localStorage.setItem("buzz:text-scale", "0.75");
    });
    await installMockBridge(page);
    await page.goto("/");

    // Confirm the zoomed-out scale actually applied to the root font size.
    await expect
      .poll(() =>
        page.evaluate(
          () => getComputedStyle(document.documentElement).fontSize,
        ),
      )
      .toBe("12px");

    expect(await firstNavButtonX(page)).toBeGreaterThanOrEqual(
      TRAFFIC_LIGHT_RIGHT_EDGE,
    );
  });
});
