import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

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

async function seedCommunities(
  page: import("@playwright/test").Page,
  communities: Array<Record<string, unknown>>,
  activeId: string,
) {
  await page.addInitScript(
    ({ list, active }) => {
      window.localStorage.setItem("buzz-communities", JSON.stringify(list));
      window.localStorage.setItem("buzz-active-community-id", active);
    },
    { list: communities, active: activeId },
  );
}

test.describe("community rail", () => {
  test("shows the rail with multiple communities despite a stale opt-out", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, {
      seedPreviewFeatures: false,
      skipCommunitySeed: true,
    });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.addInitScript((overridesKey) => {
      window.localStorage.setItem(
        overridesKey,
        JSON.stringify({ workspaceRail: false }),
      );
    }, FEATURE_OVERRIDES_STORAGE_KEY);
    await page.goto("/");

    const rail = page.getByTestId("community-rail");
    await expect(rail).toBeVisible();

    const buttonA = page.getByTestId(`community-rail-button-${COMMUNITY_A.id}`);
    const buttonB = page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`);
    await expect(buttonA).toBeVisible();
    await expect(buttonB).toBeVisible();

    // The active community is marked via aria-current.
    await expect(buttonA).toHaveAttribute("aria-current", "true");
    await expect(buttonB).not.toHaveAttribute("aria-current", "true");

    // The add-community affordance lives at the bottom of the rail.
    await expect(page.getByTestId("community-rail-add")).toBeVisible();
  });

  test("restores pointer events after dismissing community settings", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.goto("/");

    const communityButton = page.getByTestId(
      `community-rail-button-${COMMUNITY_A.id}`,
    );
    await communityButton.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Community settings" }).click();

    await expect(
      page.getByRole("dialog", { name: "Edit Community" }),
    ).toBeVisible();
    await page.mouse.click(0, 0);

    await expect(
      page.getByRole("dialog", { name: "Edit Community" }),
    ).toHaveCount(0);
    await expect(page.locator("body")).not.toHaveCSS("pointer-events", "none");
    await page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`).click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.localStorage.getItem("buzz-active-community-id"),
        ),
      )
      .toBe(COMMUNITY_B.id);
  });

  test("switches the active community on click", async ({ page }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);

    await page.goto("/");

    await page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`).click();

    // Switching persists the newly active community id (the app then remounts
    // against that relay via the existing community-init path).
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.localStorage.getItem("buzz-active-community-id"),
        ),
      )
      .toBe(COMMUNITY_B.id);
  });

  test("shows the quiet switch gate, not the boot splash, while switching", async ({
    page,
  }) => {
    // Slow down apply_workspace so the loading phase is observable.
    await installMockBridge(
      page,
      { applyCommunityDelayMs: 800 },
      { skipCommunitySeed: true },
    );
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.goto("/");

    // Cold boot still uses the full splash.
    await expect(page.getByTestId("app-loading-gate")).toBeVisible();
    const buttonB = page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`);
    await expect(buttonB).toBeVisible();

    await buttonB.click();

    // The switch renders the quiet gate; the "Setting up your community"
    // splash must not reappear.
    await expect(page.getByTestId("community-switch-gate")).toBeVisible();
    await expect(page.getByTestId("app-loading-gate")).toHaveCount(0);

    // The app settles into the new community once apply completes.
    await expect(buttonB).toHaveAttribute("aria-current", "true");
  });

  test("hides the rail with a single community", async ({ page }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A], COMMUNITY_A.id);
    await page.goto("/");

    // The channel sidebar still renders; the rail is omitted (a rail of one
    // adds nothing).
    await expect(page.getByTestId("app-sidebar")).toBeVisible();
    await expect(page.getByTestId("community-rail")).toHaveCount(0);
  });

  test("keeps the rail visible when the sidebar is collapsed", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.goto("/");

    const rail = page.getByTestId("community-rail");
    await expect(rail).toBeVisible();

    // Collapse the sidebar via its keyboard shortcut. The rail is a sibling of
    // the sidebar, not inside it, so it must stay fully visible and unshifted.
    await page.evaluate(() => {
      const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          key: "s",
          ctrlKey: !isMac,
          metaKey: isMac,
        }),
      );
    });

    await expect(rail).toBeVisible();
    await expect(
      page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`),
    ).toBeVisible();
    await expect(page.getByTestId("community-rail-add")).toBeVisible();
  });

  test("clears the macOS traffic lights", async ({ page }) => {
    // Spoof macOS so the rail applies its traffic-light top inset.
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
    });
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.goto("/");

    // The first community button must start below the traffic-light band
    // (native controls sit around y<=31 with trafficLightPosition y:24).
    const firstButton = page.getByTestId(
      `community-rail-button-${COMMUNITY_A.id}`,
    );
    await expect(firstButton).toBeVisible();
    const box = await firstButton.boundingBox();
    expect(box).not.toBeNull();
    expect(box?.y ?? 0).toBeGreaterThanOrEqual(32);

    // With the rail visible, the top-chrome controls (sidebar toggle, back/
    // forward) sit just past the traffic lights near the rail edge — not
    // shifted far right by a redundant traffic-light offset.
    const toggle = page
      .locator('[data-testid="app-top-chrome"] button')
      .first();
    const toggleBox = await toggle.boundingBox();
    expect(toggleBox).not.toBeNull();
    expect(toggleBox?.x ?? 0).toBeLessThan(120);
  });

  test("drag-to-reorder updates the stored community order and survives reload", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    // Seed only if not already set so the persisted order survives page.reload().
    await page.addInitScript(
      ({ list, active }) => {
        if (!window.localStorage.getItem("buzz-communities")) {
          window.localStorage.setItem("buzz-communities", JSON.stringify(list));
        }
        if (!window.localStorage.getItem("buzz-active-community-id")) {
          window.localStorage.setItem("buzz-active-community-id", active);
        }
      },
      { list: [COMMUNITY_A, COMMUNITY_B], active: COMMUNITY_A.id },
    );
    await page.goto("/");

    const buttonA = page.getByTestId(`community-rail-button-${COMMUNITY_A.id}`);
    const buttonB = page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`);
    await expect(buttonA).toBeVisible();
    await expect(buttonB).toBeVisible();

    // Drag B (lower) up over A (higher) so the order becomes [B, A].
    const boxA = await buttonA.boundingBox();
    const boxB = await buttonB.boundingBox();
    if (!boxA || !boxB) throw new Error("community buttons not laid out");

    const startX = boxB.x + boxB.width / 2;
    const startY = boxB.y + boxB.height / 2;
    const targetY = boxA.y + boxA.height / 2;

    // dnd-kit PointerSensor requires a 6px activation distance before it picks
    // up the drag. Move in small steps so pointermove events fire on every pixel.
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    await page.mouse.move(startX, startY - 3, { steps: 3 });
    await page.mouse.move(startX, targetY, { steps: 20 });
    await page.mouse.up();

    // The community list in localStorage must now be [B, A].
    await expect
      .poll(() =>
        page.evaluate(() => {
          const raw = window.localStorage.getItem("buzz-communities");
          if (!raw) return null;
          const list = JSON.parse(raw) as Array<{ id: string }>;
          return list.map((c) => c.id);
        }),
      )
      .toEqual([COMMUNITY_B.id, COMMUNITY_A.id]);

    // Verify the new order is also reflected in the rendered DOM — B button
    // must appear above A button.
    const newBoxA = await buttonA.boundingBox();
    const newBoxB = await buttonB.boundingBox();
    if (!newBoxA || !newBoxB)
      throw new Error("community buttons not laid out after drag");
    expect(newBoxB.y).toBeLessThan(newBoxA.y);

    // Reload and confirm the order survives restart: addInitScript is
    // conditional (no-op when data already exists), so the dragged [B, A]
    // order is what React reads on boot.
    await page.reload();
    await expect(page.getByTestId("community-rail")).toBeVisible();

    // Storage must still be [B, A] after reload.
    const storedOrder = await page.evaluate(() => {
      const raw = window.localStorage.getItem("buzz-communities");
      if (!raw) return null;
      const list = JSON.parse(raw) as Array<{ id: string }>;
      return list.map((c) => c.id);
    });
    expect(storedOrder).toEqual([COMMUNITY_B.id, COMMUNITY_A.id]);

    // DOM order must also be [B, A] after reload.
    const reloadBoxA = await buttonA.boundingBox();
    const reloadBoxB = await buttonB.boundingBox();
    if (!reloadBoxA || !reloadBoxB)
      throw new Error("community buttons not laid out after reload");
    expect(reloadBoxB.y).toBeLessThan(reloadBoxA.y);
  });

  test("keyboard reorder: Space to pick up, ArrowUp to move, Space to drop updates stored order", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
    await seedCommunities(page, [COMMUNITY_A, COMMUNITY_B], COMMUNITY_A.id);
    await page.goto("/");

    const buttonA = page.getByTestId(`community-rail-button-${COMMUNITY_A.id}`);
    const buttonB = page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`);
    await expect(buttonA).toBeVisible();
    await expect(buttonB).toBeVisible();

    // Focus B (the second/lower item) and use keyboard to move it above A.
    // Note: page.keyboard.press("Space") fires the button's native click on this
    // Chromium build even when React's onKeyDown calls preventDefault — a CDP
    // input-injection quirk. The synthetic dispatch below goes directly through
    // React's event system where preventDefault correctly suppresses the click,
    // while still exercising the real KeyboardSensor path (Thufir verified the
    // test fails when KeyboardSensor is removed).
    await buttonB.focus();
    await page.evaluate((testId) => {
      const el = document.querySelector(`[data-testid="${testId}"]`);
      if (!el) throw new Error(`button not found: ${testId}`);
      el.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: " ",
          code: "Space",
          bubbles: true,
          cancelable: true,
        }),
      );
    }, `community-rail-button-${COMMUNITY_B.id}`);
    // ArrowUp moves the active item one slot up.
    await page.keyboard.press("ArrowUp");
    // Space drops the item — same synthetic dispatch for consistency.
    await page.evaluate((testId) => {
      const el = document.querySelector(`[data-testid="${testId}"]`);
      if (!el) throw new Error(`button not found: ${testId}`);
      el.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: " ",
          code: "Space",
          bubbles: true,
          cancelable: true,
        }),
      );
    }, `community-rail-button-${COMMUNITY_B.id}`);

    // The community list in localStorage must now be [B, A].
    await expect
      .poll(() =>
        page.evaluate(() => {
          const raw = window.localStorage.getItem("buzz-communities");
          if (!raw) return null;
          const list = JSON.parse(raw) as Array<{ id: string }>;
          return list.map((c) => c.id);
        }),
      )
      .toEqual([COMMUNITY_B.id, COMMUNITY_A.id]);
  });
});
