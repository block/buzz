import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/sidebar-offcanvas-rail";
const THEME_STORAGE_KEY = "buzz-theme";
const RELAY_URL = "ws://localhost:3000";

const COMMUNITY_A = {
  id: "ws-a",
  name: "Alpha",
  relayUrl: RELAY_URL,
  addedAt: "2026-01-01T00:00:00.000Z",
};
const COMMUNITY_B = {
  id: "ws-b",
  name: "Bravo",
  relayUrl: "ws://localhost:3001",
  addedAt: "2026-01-02T00:00:00.000Z",
};

async function setup(page: Page, theme: string) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(
    ({ key, value }) => {
      window.localStorage.setItem(key, value);
    },
    { key: THEME_STORAGE_KEY, value: theme },
  );
  await installMockBridge(page, undefined, { skipCommunitySeed: true });
  await page.addInitScript(
    ({ list, active }) => {
      window.localStorage.setItem("buzz-communities", JSON.stringify(list));
      window.localStorage.setItem("buzz-active-community-id", active);
    },
    { list: [COMMUNITY_A, COMMUNITY_B], active: COMMUNITY_A.id },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("community-rail")).toBeVisible();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

/**
 * Regression: the app-sidebar layer is overflow-visible (huddle drawer), so
 * the offcanvas-collapsed sidebar slides out of its container but kept
 * painting over the community rail — opaquely on flat themes, as ghost
 * fragments on the transparent Buzz chrome. The collapsed sidebar must be
 * invisible and non-interactive, leaving the rail clean in every theme.
 */
for (const theme of ["buzz", "buzz-dark", "vesper"]) {
  test(`collapsed sidebar leaves the community rail clean — ${theme}`, async ({
    page,
  }) => {
    await setup(page, theme);
    await page.screenshot({ path: `${SHOTS}/${theme}-expanded.png` });

    await page.locator('[data-sidebar="trigger"]').first().click();
    const shell = page.locator(
      '[data-state="collapsed"][data-collapsible="offcanvas"]',
    );
    await expect(shell).toHaveCount(1);
    // Let the 200ms slide finish; visibility flips at the transition's end.
    await page.waitForTimeout(500);

    // Second direct child = the sliding sidebar container (first is the gap).
    const offscreenSidebar = shell.locator("> div").nth(1);
    await expect(offscreenSidebar).toHaveCSS("visibility", "hidden");
    await expect(offscreenSidebar).toHaveCSS("pointer-events", "none");

    // The community rail stays visible and interactive beneath it.
    await expect(page.getByTestId("community-rail")).toBeVisible();
    await expect(
      page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`),
    ).toBeVisible();
    await page.screenshot({ path: `${SHOTS}/${theme}-collapsed.png` });
  });
}
