import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Glass background must clear the community rail's opaque `bg-sidebar` paint
 * so the native vibrancy shows through the whole navigation column, not just
 * the channel sidebar. Regression test for the rail being left out of the
 * `:root[data-glass-background]` transparency list (theme.css).
 */
async function seedGlassOnMac(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "github-light");
    window.localStorage.setItem("buzz-glass-background", "true");
    (window as typeof window & { isTauri?: boolean }).isTauri = true;
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "MacIntel",
    });
  });
}

/** Registered AFTER the bridge so it can append to the seeded community list. */
async function appendSecondCommunity(page: Page) {
  await page.addInitScript(() => {
    const raw = window.localStorage.getItem("buzz-communities");
    const list = raw ? (JSON.parse(raw) as Array<unknown>) : [];
    list.push({
      id: "ws-b",
      name: "Bravo",
      relayUrl: "ws://localhost:3001",
      addedAt: "2026-01-02T00:00:00.000Z",
    });
    window.localStorage.setItem("buzz-communities", JSON.stringify(list));
  });
}

test("glass background clears the community rail surface", async ({ page }) => {
  await seedGlassOnMac(page);
  await installMockBridge(page);
  await appendSecondCommunity(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(page.locator("html")).toHaveAttribute(
    "data-glass-background",
    "",
  );

  const rail = page.getByTestId("community-rail");
  await expect(rail).toBeVisible();
  await expect(rail).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
});
