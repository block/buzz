import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const RELAY_URL = "ws://localhost:3000";

const WORKSPACE_A = {
  id: "ws-a",
  name: "Alpha",
  relayUrl: RELAY_URL,
  addedAt: "2026-01-01T00:00:00.000Z",
};
const WORKSPACE_B = {
  id: "ws-b",
  name: "Bravo",
  relayUrl: "ws://localhost:3001",
  addedAt: "2026-01-02T00:00:00.000Z",
};

async function seedWorkspaces(
  page: import("@playwright/test").Page,
  workspaces: Array<Record<string, unknown>>,
  activeId: string,
) {
  await page.addInitScript(
    ({ list, active }) => {
      window.localStorage.setItem("buzz-workspaces", JSON.stringify(list));
      window.localStorage.setItem("buzz-active-workspace-id", active);
    },
    { list: workspaces, active: activeId },
  );
}

test.describe("workspace rail", () => {
  test("shows a button per workspace and highlights the active one", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipWorkspaceSeed: true });
    await seedWorkspaces(page, [WORKSPACE_A, WORKSPACE_B], WORKSPACE_A.id);
    await page.goto("/");

    const rail = page.getByTestId("workspace-rail");
    await expect(rail).toBeVisible();

    const buttonA = page.getByTestId(`workspace-rail-button-${WORKSPACE_A.id}`);
    const buttonB = page.getByTestId(`workspace-rail-button-${WORKSPACE_B.id}`);
    await expect(buttonA).toBeVisible();
    await expect(buttonB).toBeVisible();

    // The active workspace is marked via aria-current.
    await expect(buttonA).toHaveAttribute("aria-current", "true");
    await expect(buttonB).not.toHaveAttribute("aria-current", "true");

    // The add-workspace affordance lives at the bottom of the rail.
    await expect(page.getByTestId("workspace-rail-add")).toBeVisible();
  });

  test("switches the active workspace on click", async ({ page }) => {
    await installMockBridge(page, undefined, { skipWorkspaceSeed: true });
    await seedWorkspaces(page, [WORKSPACE_A, WORKSPACE_B], WORKSPACE_A.id);
    await page.goto("/");

    await page.getByTestId(`workspace-rail-button-${WORKSPACE_B.id}`).click();

    // Switching persists the newly active workspace id (the app then remounts
    // against that relay via the existing workspace-init path).
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.localStorage.getItem("buzz-active-workspace-id"),
        ),
      )
      .toBe(WORKSPACE_B.id);
  });

  test("shows the quiet switch gate, not the boot splash, while switching", async ({
    page,
  }) => {
    // Slow down apply_workspace so the loading phase is observable.
    await installMockBridge(
      page,
      { applyWorkspaceDelayMs: 800 },
      { skipWorkspaceSeed: true },
    );
    await seedWorkspaces(page, [WORKSPACE_A, WORKSPACE_B], WORKSPACE_A.id);
    await page.goto("/");

    // Cold boot still uses the full splash.
    await expect(page.getByTestId("app-loading-gate")).toBeVisible();
    const buttonB = page.getByTestId(`workspace-rail-button-${WORKSPACE_B.id}`);
    await expect(buttonB).toBeVisible();

    await buttonB.click();

    // The switch renders the quiet gate; the "Setting up your workspace"
    // splash must not reappear.
    await expect(page.getByTestId("workspace-switch-gate")).toBeVisible();
    await expect(page.getByTestId("app-loading-gate")).toHaveCount(0);

    // The app settles into the new workspace once apply completes.
    await expect(buttonB).toHaveAttribute("aria-current", "true");
  });

  test("hides the rail with a single workspace", async ({ page }) => {
    await installMockBridge(page, undefined, { skipWorkspaceSeed: true });
    await seedWorkspaces(page, [WORKSPACE_A], WORKSPACE_A.id);
    await page.goto("/");

    // The channel sidebar still renders; the rail is omitted (a rail of one
    // adds nothing).
    await expect(page.getByTestId("app-sidebar")).toBeVisible();
    await expect(page.getByTestId("workspace-rail")).toHaveCount(0);
  });

  test("keeps the rail visible when the sidebar is collapsed", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { skipWorkspaceSeed: true });
    await seedWorkspaces(page, [WORKSPACE_A, WORKSPACE_B], WORKSPACE_A.id);
    await page.goto("/");

    const rail = page.getByTestId("workspace-rail");
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
      page.getByTestId(`workspace-rail-button-${WORKSPACE_B.id}`),
    ).toBeVisible();
    await expect(page.getByTestId("workspace-rail-add")).toBeVisible();
  });

  test("clears the macOS traffic lights", async ({ page }) => {
    // Spoof macOS so the rail applies its traffic-light top inset.
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
    });
    await installMockBridge(page, undefined, { skipWorkspaceSeed: true });
    await seedWorkspaces(page, [WORKSPACE_A, WORKSPACE_B], WORKSPACE_A.id);
    await page.goto("/");

    // The first workspace button must start below the traffic-light band
    // (native controls sit around y<=31 with trafficLightPosition y:24).
    const firstButton = page.getByTestId(
      `workspace-rail-button-${WORKSPACE_A.id}`,
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
});
