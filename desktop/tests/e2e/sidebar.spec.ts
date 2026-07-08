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

// Regression guard for the "Leave channel" lockup: opening a modal AlertDialog
// from a modal Radix ContextMenu leaves `pointer-events: none` stuck on <body>
// after the dialog closes, freezing the whole app. The fix makes the sidebar
// context menus non-modal. This asserts the app is still interactive.
async function expectAppClickable(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.body).pointerEvents),
    )
    .not.toBe("none");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
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

test("leaving a channel from the context menu never freezes the app", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // Cancel path: dialog opens from the context menu, then is dismissed.
  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Leave channel" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expectAppClickable(page);

  // Confirm path: same overlay lifecycle, plus the leave mutation.
  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Leave channel" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("button", { name: "Leave" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expectAppClickable(page);
});

test("fades the pinned sidebar chrome edges", async ({ page }) => {
  await page.goto("/");

  const pinnedHeader = page.getByTestId("sidebar-pinned-header");
  const footer = page.locator(
    '[data-testid="app-sidebar"] [data-sidebar="footer"]',
  );
  const channelContent = page.getByTestId("sidebar-channel-content");
  await expect(pinnedHeader).toBeVisible();
  await expect(footer).toBeVisible();
  await expect(channelContent).toBeVisible();

  const fadeStyles = await page.evaluate(() => {
    const header = document.querySelector<HTMLElement>(
      '[data-testid="app-sidebar"] [data-testid="sidebar-pinned-header"]',
    );
    const footerElement = document.querySelector<HTMLElement>(
      '[data-testid="app-sidebar"] [data-sidebar="footer"]',
    );
    const channelElement = document.querySelector<HTMLElement>(
      '[data-testid="sidebar-channel-content"]',
    );

    if (!header || !footerElement || !channelElement) {
      throw new Error("Expected sidebar chrome elements to be rendered");
    }

    const headerBefore = getComputedStyle(header, "::before");
    const headerStyle = getComputedStyle(header);
    const sidebarElement = header.closest<HTMLElement>(
      '[data-sidebar="sidebar"]',
    );
    const sidebarStyle = sidebarElement
      ? getComputedStyle(sidebarElement)
      : null;
    const footerStyle = getComputedStyle(footerElement);
    const footerBefore = getComputedStyle(footerElement, "::before");
    const channelBefore = getComputedStyle(channelElement, "::before");
    const channelAfter = getComputedStyle(channelElement, "::after");
    const headerRect = header.getBoundingClientRect();
    const footerRect = footerElement.getBoundingClientRect();

    return {
      channelAfterBackground: channelAfter.backgroundImage,
      channelBeforeBackground: channelBefore.backgroundImage,
      footerBackgroundColor: footerStyle.backgroundColor,
      footerBackdropFilter: footerBefore.backdropFilter,
      footerBackground: footerBefore.backgroundImage,
      footerBoxShadow: footerStyle.boxShadow,
      footerFadeBoxShadow: footerBefore.boxShadow,
      footerFadeHeight: Number.parseFloat(footerBefore.height),
      footerHeight: footerRect.height,
      footerPointerEvents: footerBefore.pointerEvents,
      footerPosition: footerBefore.position,
      footerTopPx: Number.parseFloat(footerBefore.top),
      footerZIndex: footerBefore.zIndex,
      headerBackground: headerBefore.backgroundImage,
      headerBackdropFilter: headerBefore.backdropFilter,
      headerBackgroundColor: headerStyle.backgroundColor,
      headerBottomPx: Number.parseFloat(headerBefore.bottom),
      headerBoxShadow: headerStyle.boxShadow,
      headerFadeBoxShadow: headerBefore.boxShadow,
      headerFadeHeight: Number.parseFloat(headerBefore.height),
      headerHeight: headerRect.height,
      headerPointerEvents: headerBefore.pointerEvents,
      headerPosition: headerBefore.position,
      headerZIndex: headerBefore.zIndex,
      sidebarBackgroundColor: sidebarStyle?.backgroundColor ?? null,
    };
  });

  expect(fadeStyles.headerBackground).toContain("gradient");
  expect(fadeStyles.headerBackgroundColor).toBe(
    fadeStyles.sidebarBackgroundColor,
  );
  expect(fadeStyles.headerBackground).toContain("rgba");
  expect(fadeStyles.headerBackground).toContain("0) 100%");
  expect(fadeStyles.headerBackdropFilter).toBe("none");
  expect(fadeStyles.headerBottomPx).toBeLessThan(0);
  expect(fadeStyles.headerBoxShadow).toBe("none");
  expect(fadeStyles.headerFadeBoxShadow).toBe("none");
  expect(fadeStyles.headerFadeHeight).toBeLessThanOrEqual(10);
  expect(fadeStyles.headerPointerEvents).toBe("none");
  expect(fadeStyles.headerPosition).toBe("absolute");
  expect(fadeStyles.headerZIndex).toBe("5");
  expect(fadeStyles.footerBackground).toContain("gradient");
  expect(fadeStyles.footerBackgroundColor).toBe(
    fadeStyles.sidebarBackgroundColor,
  );
  expect(fadeStyles.footerBackground).toContain("rgba");
  expect(fadeStyles.footerBackground).toContain("0) 100%");
  expect(fadeStyles.footerBackdropFilter).toBe("none");
  expect(fadeStyles.footerBoxShadow).toBe("none");
  expect(fadeStyles.footerFadeBoxShadow).toBe("none");
  expect(fadeStyles.footerFadeHeight).toBeLessThanOrEqual(10);
  expect(fadeStyles.footerPointerEvents).toBe("none");
  expect(fadeStyles.footerPosition).toBe("absolute");
  expect(fadeStyles.footerTopPx).toBeLessThan(0);
  expect(fadeStyles.footerZIndex).toBe("5");
  expect(fadeStyles.channelBeforeBackground).toBe("none");
  expect(fadeStyles.channelAfterBackground).toBe("none");
});

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

// Regression test for the Linux .deb auto-update guard (PR #1535).
// When auto-update is not supported (e.g. Linux .deb install), the update
// check must surface a "manual-required" card with a GitHub link and
// AppImage hint, and must NEVER invoke plugin:updater|download_and_install.
test("shows manual-required update card and never auto-downloads on non-AppImage installs", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // Override the bridge to report an update available AND auto-update not
  // supported. The mock is mutated after page load so the window object is
  // live (mirrors the ready-card test pattern).
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: {
        mock?: { updateAvailable?: boolean; autoUpdateSupported?: boolean };
      };
    };
    testWindow.__BUZZ_E2E__ = {
      ...(testWindow.__BUZZ_E2E__ ?? {}),
      mock: {
        ...(testWindow.__BUZZ_E2E__?.mock ?? {}),
        updateAvailable: true,
        autoUpdateSupported: false,
      },
    };
  });

  await page.getByTestId("sidebar-profile-card").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-updates").click();
  await page.getByRole("button", { name: "Check for Updates" }).click();

  // Settings panel shows the manual-required state, not "ready".
  await expect(page.getByTestId("settings-panel-updates")).toContainText(
    "In-app updates aren't supported on this Linux package",
  );
  await expect(page.getByTestId("settings-panel-updates")).toContainText(
    "AppImage",
  );

  await page.getByTestId("settings-back-to-app").click();

  // Sidebar card shows the manual update card.
  const updateCard = page.getByTestId("sidebar-update-card-manual");
  await expect(updateCard).toBeVisible();
  await expect(updateCard).toContainText("AppImage");

  // download_and_install must NEVER have been called.
  const commands = await page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMANDS__?: string[];
        }
      ).__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).not.toContain("plugin:updater|download_and_install");
});
