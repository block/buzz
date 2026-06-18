import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SIDEBAR_WIDTH_STORAGE_KEY = "buzz-sidebar-width";
const DEFAULT_SIDEBAR_WIDTH = 300;

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function sidebarWidth(page: Page) {
  return page.getByTestId("app-sidebar").evaluate((element) => {
    return Math.round(element.getBoundingClientRect().width);
  });
}

async function storedSidebarWidth(page: Page) {
  return page.evaluate(
    (key) => localStorage.getItem(key),
    SIDEBAR_WIDTH_STORAGE_KEY,
  );
}

async function dragSidebarRail(page: Page, deltaX: number) {
  const sidebarRail = page.locator('[data-sidebar="rail"]');
  await expect(sidebarRail).toBeVisible();
  await expect(sidebarRail).toBeEnabled();

  const box = await sidebarRail.boundingBox();
  expect(box).not.toBeNull();

  if (!box) return;

  const startX = box.x + box.width / 2;
  const startY = box.y + box.height / 2;

  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + deltaX, startY, { steps: 8 });
  await page.mouse.up();
}

test("resizes, persists, and snaps to the default sidebar width", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await expect.poll(() => sidebarWidth(page)).toBe(DEFAULT_SIDEBAR_WIDTH);
  await expect.poll(() => storedSidebarWidth(page)).toBeNull();

  await dragSidebarRail(page, 64);

  await expect.poll(() => sidebarWidth(page)).toBe(364);
  await expect.poll(() => storedSidebarWidth(page)).toBe("364");

  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect.poll(() => sidebarWidth(page)).toBe(364);

  await dragSidebarRail(page, -60);

  await expect.poll(() => sidebarWidth(page)).toBe(DEFAULT_SIDEBAR_WIDTH);
  await expect
    .poll(() => storedSidebarWidth(page))
    .toBe(String(DEFAULT_SIDEBAR_WIDTH));
});

test("shows a sidebar update card when an update is ready", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { updateAvailable?: boolean } };
    };

    testWindow.__BUZZ_E2E__ = {
      ...(testWindow.__BUZZ_E2E__ ?? {}),
      mock: {
        ...(testWindow.__BUZZ_E2E__?.mock ?? {}),
        restartDelayMs: 500,
        updateAvailable: true,
      },
    };
  });

  await page.getByTestId("sidebar-profile-card").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-updates").click();
  await page.getByRole("button", { name: "Check for Updates" }).click();
  await expect(page.getByTestId("settings-panel-updates")).toContainText(
    "Update installed. Restart to apply.",
  );

  await page.getByTestId("settings-back-to-app").click();

  const updateCard = page.getByTestId("sidebar-update-card");
  await expect(updateCard).toBeVisible();
  await expect(updateCard).toContainText("Ready to update!");
  await expect(updateCard).toContainText("Click to restart");
  await expect(page.getByTestId("sidebar-update-restart")).toBeVisible();
  const reservedCardHeight = await updateCard.evaluate(
    (element) => (element as HTMLElement).offsetHeight,
  );

  await page.getByTestId("sidebar-update-restart").click();
  await expect(updateCard).toContainText("Restarting");
  await expect(page.getByTestId("sidebar-update-restart")).toBeDisabled();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMANDS__?: string[];
            }
          ).__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("plugin:process|restart");

  const dismissButton = page.getByTestId("sidebar-update-dismiss");
  await updateCard.hover();
  const dismissButtonBox = await dismissButton.boundingBox();
  expect(dismissButtonBox).not.toBeNull();
  if (!dismissButtonBox) return;

  await page.mouse.move(
    dismissButtonBox.x + dismissButtonBox.width / 2,
    dismissButtonBox.y + dismissButtonBox.height / 2,
  );
  await page.mouse.down();
  await expect(page.locator(".buzz-poof-burst")).toHaveCount(1);
  await expect(updateCard).toBeVisible();
  await page.mouse.up();
  await expect(updateCard).toHaveAttribute("data-dismissing", "true");
  await expect
    .poll(() =>
      updateCard.evaluate((element) => (element as HTMLElement).offsetHeight),
    )
    .toBe(reservedCardHeight);
  await expect
    .poll(() =>
      updateCard.evaluate((element) =>
        Number.parseFloat(getComputedStyle(element).opacity),
      ),
    )
    .toBeLessThan(0.05);
  await expect(updateCard).toBeHidden();
});
