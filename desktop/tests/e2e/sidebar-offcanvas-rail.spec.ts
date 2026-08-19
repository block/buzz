import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

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
  await page.setViewportSize({ width: 960, height: 540 });
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
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/${theme}-expanded.png` });

    const communityRail = page.getByTestId("community-rail");
    const communityButton = page.getByTestId(
      `community-rail-button-${COMMUNITY_B.id}`,
    );
    const railBoxBeforeCollapse = await communityRail.boundingBox();
    expect(railBoxBeforeCollapse).not.toBeNull();

    // Observe the transition before triggering it, then hold every animated
    // sidebar-content property at its midpoint. This keeps the regression
    // causal without making its assertions depend on Playwright or rAF
    // scheduler latency.
    const transition = await communityButton.evaluate(async (button) => {
      const rail = button.closest('[data-testid="community-rail"]');
      const trigger = document.querySelector<HTMLElement>(
        '[data-sidebar="trigger"]',
      );
      const sidebarContent = document.querySelector<HTMLElement>(
        "[data-sidebar-transition-content]",
      );
      if (!(rail instanceof HTMLElement) || !trigger || !sidebarContent) {
        return null;
      }

      const transitionStarted = new Promise<void>((resolve) => {
        sidebarContent.addEventListener("transitionrun", () => resolve(), {
          once: true,
        });
      });
      trigger.click();
      await transitionStarted;

      const animations = sidebarContent.getAnimations();
      await Promise.all(animations.map((animation) => animation.ready));
      for (const animation of animations) {
        animation.pause();
        animation.currentTime = 100;
      }

      const buttonBox = button.getBoundingClientRect();
      const railBox = rail.getBoundingClientRect();
      const hit = document.elementFromPoint(
        buttonBox.x + buttonBox.width / 2,
        buttonBox.y + buttonBox.height / 2,
      );
      const railStyle = getComputedStyle(rail);
      const sidebarStyle = getComputedStyle(sidebarContent);
      const result = {
        durations: animations.map(
          (animation) => animation.effect?.getTiming().duration,
        ),
        hitRail: hit === rail || rail.contains(hit),
        opacity: railStyle.opacity,
        sidebarOpacity: Number.parseFloat(sidebarStyle.opacity),
        sidebarScale: sidebarStyle.scale,
        sidebarTranslateX: Number.parseFloat(sidebarStyle.translate),
        visibility: railStyle.visibility,
        x: railBox.x,
        y: railBox.y,
      };

      for (const animation of animations) animation.finish();
      return result;
    });
    expect(transition).not.toBeNull();
    expect(transition?.durations).toEqual([200, 200, 200]);
    expect(transition).toMatchObject({
      hitRail: true,
      opacity: "1",
      visibility: "visible",
      x: railBoxBeforeCollapse?.x,
      y: railBoxBeforeCollapse?.y,
    });
    expect(transition?.sidebarOpacity).toBeGreaterThan(0);
    expect(transition?.sidebarOpacity).toBeLessThan(1);
    expect(transition?.sidebarScale).not.toBe("none");
    expect(transition?.sidebarScale).not.toBe("0.95");
    expect(transition?.sidebarTranslateX).toBeGreaterThan(0);
    expect(transition?.sidebarTranslateX).toBeLessThan(24);

    const shell = page.locator(
      '[data-state="collapsed"][data-collapsible="offcanvas"]',
    );
    await expect(shell).toHaveCount(1);

    // Let the 200ms slide finish; visibility flips at the transition's end.
    await page.waitForTimeout(250);

    // Second direct child = the sliding sidebar container (first is the gap).
    const offscreenSidebar = shell.locator("> div").nth(1);
    await expect(offscreenSidebar).toHaveCSS("visibility", "hidden");
    await expect(offscreenSidebar).toHaveCSS("pointer-events", "none");

    // The community rail stays visible and interactive beneath it.
    await expect(page.getByTestId("community-rail")).toBeVisible();
    await expect(
      page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/${theme}-collapsed.png` });
  });
}
