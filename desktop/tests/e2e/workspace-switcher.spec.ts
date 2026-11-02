import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const PUBKEY = "deadbeef".repeat(8);
const WORKSPACES = [
  {
    id: "workspace-one",
    name: "Colony",
    relayUrl: "wss://colony.colony.ainative.ventures",
    pubkey: PUBKEY,
    addedAt: "2026-01-01T00:00:00.000Z",
  },
  {
    id: "workspace-two",
    name: "Horizon Labs",
    relayUrl: "wss://horizon-labs.colony.ainative.ventures",
    pubkey: PUBKEY,
    addedAt: "2026-01-02T00:00:00.000Z",
  },
];
const CAPTURE_DIR = process.env.WS_SWITCHER_CAPTURE_DIR;
const CAPTURE_PHASE = process.env.WS_SWITCHER_CAPTURE_PHASE ?? "capture";
const WORKSPACE_ICON = `data:image/svg+xml,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" rx="12" fill="#174F38"/><circle cx="20" cy="20" r="8" fill="#F5F1D9"/></svg>',
)}`;

async function seedWorkspaces(
  page: Page,
  options: { theme?: "light" | "dark"; workspaceCount?: number } = {},
) {
  const workspaces = WORKSPACES.slice(0, options.workspaceCount ?? 2);
  await page.addInitScript(
    ({ workspaces, theme, icon }) => {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        get: () => "MacIntel",
      });
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(workspaces),
      );
      window.localStorage.setItem("buzz-active-community-id", workspaces[0].id);
      window.localStorage.setItem(
        "buzz-community-icons",
        JSON.stringify(
          Object.fromEntries(
            workspaces.map((workspace, index) => [
              workspace.relayUrl,
              index === 0 ? icon : null,
            ]),
          ),
        ),
      );
      window.localStorage.setItem(
        "buzz-theme",
        theme === "dark" ? "buzz-dark" : "buzz",
      );
      window.localStorage.setItem("buzz-follow-system", "false");
      window.localStorage.setItem("buzz-accent-color", "#5B45BE");
    },
    { workspaces, theme: options.theme ?? "light", icon: WORKSPACE_ICON },
  );
  await installMockBridge(page, undefined, { skipCommunitySeed: true });
}

async function dispatchWorkspaceShortcut(page: Page, digit: string) {
  const cancelled = await page.evaluate((key) => {
    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key,
      metaKey: true,
    });
    const wasNotCancelled = window.dispatchEvent(event);
    return !wasNotCancelled && event.defaultPrevented;
  }, digit);
  expect(cancelled).toBe(true);
}

async function openWorkspaceHeader(page: Page) {
  const switcher = page.getByTestId("sidebar-business-switcher");
  const viewportWidth = page.viewportSize()?.width ?? 1440;
  if (viewportWidth < 768) {
    await expect(page.getByTestId("mobile-workspace-switcher")).toBeVisible();
    return;
  } else {
    const appSidebar = page.getByTestId("app-sidebar");
    if ((await appSidebar.getAttribute("data-state")) === "collapsed") {
      await appSidebar.getByRole("button", { name: "Toggle Sidebar" }).click();
      await expect(appSidebar).toHaveAttribute("data-state", "expanded");
    }
  }
  await expect(switcher).toBeVisible();
  await expect(switcher.locator(".sidebar-company-name")).toBeVisible();
}

async function capture(
  page: Page,
  filename: string,
  options: {
    clip?: { x: number; y: number; width: number; height: number };
  } = {},
) {
  if (!CAPTURE_DIR) return;
  mkdirSync(CAPTURE_DIR, { recursive: true });
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: join(CAPTURE_DIR, `${CAPTURE_PHASE}-${filename}.png`),
    ...options,
  });
}

async function openWorkspaceMenu(page: Page) {
  await page.getByTestId("sidebar-business-switcher").click();
  const menu = page.getByTestId("workspace-switcher-menu");
  await expect(menu).toBeVisible();
  return menu;
}

test("header menu lists workspaces, switches by click and shortcut, profile stays separate", async ({
  page,
}) => {
  await seedWorkspaces(page);
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await expect(page.getByTestId("community-rail")).toBeVisible();

  const menu = await openWorkspaceMenu(page);
  const firstWorkspace = menu.getByTestId("workspace-menu-item-workspace-one");
  const secondWorkspace = menu.getByTestId("workspace-menu-item-workspace-two");
  await expect(firstWorkspace).toContainText("Colony");
  await expect(firstWorkspace).toContainText("colony.colony.ainative.ventures");
  await expect(
    menu.getByTestId("workspace-menu-mark-workspace-one"),
  ).toHaveAttribute("data-source", "community");
  await expect(firstWorkspace).toContainText("⌘1");
  await expect(firstWorkspace).toHaveAttribute("aria-current", "true");
  await expect(
    menu.getByRole("menuitem", {
      name: "Open workspace Colony, colony.colony.ainative.ventures, current workspace, ⌘1",
      exact: true,
    }),
  ).toHaveCount(1);
  await expect(secondWorkspace).toContainText("Horizon Labs");
  await expect(secondWorkspace).toContainText("⌘2");
  await expect(secondWorkspace).not.toHaveAttribute("aria-current", "true");
  await expect(menu.getByTestId("workspace-leave-action")).toBeVisible();
  await capture(page, "WSMenuTwo");

  await page.keyboard.press("Escape");
  await page.getByTestId("sidebar-business-switcher").focus();
  await page.keyboard.press("Enter");
  await expect(menu).toBeVisible();
  await expect(firstWorkspace).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(secondWorkspace).toBeFocused();
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("buzz-active-community-id")),
    )
    .toBe("workspace-two");
  await expect(page.getByTestId("sidebar-business-switcher")).toContainText(
    "Horizon Labs",
  );
  // Chromium reserves Meta+number for tab switching, so dispatch the same keydown at the app window.
  await dispatchWorkspaceShortcut(page, "1");
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("buzz-active-community-id")),
    )
    .toBe("workspace-one");
  await dispatchWorkspaceShortcut(page, "2");
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("buzz-active-community-id")),
    )
    .toBe("workspace-two");
  await expect(page.getByTestId("sidebar-business-switcher")).toContainText(
    "Horizon Labs",
  );
  const reopenedMenu = await openWorkspaceMenu(page);
  await reopenedMenu.getByTestId("workspace-menu-item-workspace-one").click();
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("buzz-active-community-id")),
    )
    .toBe("workspace-one");
  await expect(page.getByTestId("sidebar-business-switcher")).toContainText(
    "Colony",
  );
  await page.getByTestId("sidebar-profile-avatar-button").click();
  await expect(page.getByTestId("profile-popover")).toBeVisible();
  await expect(page.getByTestId("profile-popover-community")).toHaveCount(0);
});

test("light header menu fits a single workspace", async ({ page }) => {
  await seedWorkspaces(page, { workspaceCount: 1 });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await expect(page.getByTestId("community-rail")).toHaveCount(0);
  const menu = await openWorkspaceMenu(page);
  await expect(
    menu.getByTestId("workspace-menu-item-workspace-one"),
  ).toBeVisible();
  await expect(
    menu.getByTestId("workspace-menu-item-workspace-two"),
  ).toHaveCount(0);
  await capture(page, "WSMenu");
});

test("single workspace sidebar keeps the workspace header", async ({
  page,
}) => {
  await seedWorkspaces(page, { workspaceCount: 1 });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await expect(page.getByTestId("community-rail")).toHaveCount(0);
  await expect(page.getByTestId("sidebar-business-switcher")).toBeVisible();
  await capture(page, "WSSidebar", {
    clip: { x: 0, y: 0, width: 280, height: 960 },
  });
});

test("dark header menu preserves workspace rows and shortcut hints", async ({
  page,
}) => {
  await seedWorkspaces(page, { theme: "dark", workspaceCount: 1 });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await expect(page.getByTestId("community-rail")).toHaveCount(0);
  const menu = await openWorkspaceMenu(page);
  await expect(
    menu.getByTestId("workspace-menu-item-workspace-one"),
  ).toBeVisible();
  await expect(
    menu.getByTestId("workspace-menu-item-workspace-two"),
  ).toHaveCount(0);
  await capture(page, "WSMenuDark");
});

test("workspace rail keeps the add action and opens create", async ({
  page,
}) => {
  await seedWorkspaces(page);
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.route("**/api/communities/config", (route) =>
    route.fulfill({
      json: {
        self_serve: true,
        domain: "colony.ainative.ventures",
        public: true,
        max_per_owner: 3,
      },
    }),
  );
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await expect(page.getByTestId("community-rail")).toBeVisible();
  const secondWorkspaceInRail = page.getByTestId(
    "community-rail-button-workspace-two",
  );
  await expect(secondWorkspaceInRail).toHaveAccessibleName("Horizon Labs, ⌘2");
  await secondWorkspaceInRail.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Horizon Labs, ⌘2");
  const railCreateAction = page.getByTestId("community-rail-add");
  await expect(railCreateAction).toHaveAccessibleName("Create a workspace");
  await capture(page, "WSRail");
  await railCreateAction.click();
  const railCreateDialog = page.getByTestId("workspace-action-dialog");
  await expect(railCreateDialog).toBeVisible();
  await expect(
    railCreateDialog.getByRole("heading", { name: "Create a workspace" }),
  ).toBeVisible();
  await railCreateDialog.getByTestId("workspace-dialog-close").click();
});

test("workspace create checks the auto address without submitting", async ({
  page,
}) => {
  await seedWorkspaces(page, { workspaceCount: 1 });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.route("**/api/communities/config", (route) =>
    route.fulfill({
      json: {
        self_serve: true,
        domain: "colony.ainative.ventures",
        public: true,
        max_per_owner: 3,
      },
    }),
  );
  await page.route("**/api/communities/availability**", (route) => {
    const name = new URL(route.request().url()).searchParams.get("name") ?? "";
    return route.fulfill({ json: { name, available: true } });
  });
  const createPosts: string[] = [];
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname === "/api/communities"
    ) {
      createPosts.push(request.url());
    }
  });

  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  const menu = await openWorkspaceMenu(page);
  await menu.getByTestId("workspace-create-action").click();
  const dialog = page.getByTestId("workspace-action-dialog");
  await expect(dialog).toBeVisible();
  await page.getByTestId("workspace-create-name").fill("Horizon Labs");
  await expect(page.getByTestId("workspace-create-address")).toHaveValue(
    "horizon-labs",
  );
  await expect(page.getByTestId("workspace-create-availability")).toContainText(
    "That address is available.",
  );
  await page
    .getByTestId("workspace-create-form")
    .getByRole("heading", { name: "Create a workspace" })
    .click();
  await capture(page, "WSCreate");
  await expect(page.getByTestId("workspace-create-submit")).toBeEnabled();
  expect(createPosts).toEqual([]);
});

test("workspace settings start collapsed and expose accurate advanced fields", async ({
  page,
}) => {
  await seedWorkspaces(page, { workspaceCount: 1 });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  const menu = await openWorkspaceMenu(page);
  await menu.getByTestId("workspace-settings-action").click();
  const settings = page.getByRole("dialog");
  await expect(settings.getByText("Workspace settings")).toBeVisible();
  const advanced = page.getByTestId("workspace-settings-advanced-toggle");
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(
    page.getByTestId("workspace-settings-advanced-fields"),
  ).toHaveCount(0);
  await capture(page, "WSSettings");
  await advanced.click();
  await expect(
    page.getByRole("textbox", { name: "Connection address" }),
  ).toBeVisible();
  await expect(page.getByLabel("API token (optional)")).toHaveAttribute(
    "placeholder",
    "Paste a token",
  );
  await expect(page.getByLabel("API token (optional)")).not.toHaveValue(
    /buzz_/,
  );
  await page.setViewportSize({ width: 1440, height: 1260 });
  await capture(page, "WSSettingsAdvanced");
  await page.getByRole("button", { name: "Close", exact: true }).click();
});

test("light mobile workspace switcher opens as a sheet", async ({ page }) => {
  await seedWorkspaces(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await page.getByTestId("mobile-workspace-switcher").click();
  await expect(page.getByTestId("community-rail")).toHaveCount(0);
  await expect(page.getByTestId("mobile-workspace-mark")).toHaveAttribute(
    "data-source",
    "community",
  );
  const sheet = page.getByTestId("workspace-switcher-sheet");
  await expect(sheet).toBeVisible();
  await expect(sheet.getByText("Switch workspace")).toBeVisible();
  await expect(
    sheet.getByTestId("workspace-menu-item-workspace-one"),
  ).toBeVisible();
  await expect(
    sheet.getByTestId("workspace-menu-item-workspace-two"),
  ).toBeVisible();
  await expect(
    sheet.getByTestId("workspace-menu-item-workspace-one"),
  ).toHaveAttribute("aria-current", "true");
  await expect(sheet).not.toContainText("⌘1");
  await expect(sheet).not.toContainText("⌘2");
  await expect(sheet.getByTestId("workspace-leave-action")).toHaveCount(0);
  await capture(page, "WSMobileMenu");
});

test("dark mobile workspace menu fits the 390px sheet", async ({ page }) => {
  await seedWorkspaces(page, { theme: "dark" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/#/today");
  await openWorkspaceHeader(page);
  await page.getByTestId("mobile-workspace-switcher").click();
  await expect(page.getByTestId("community-rail")).toHaveCount(0);
  const sheet = page.getByTestId("workspace-switcher-sheet");
  await expect(sheet).toBeVisible();
  await expect(
    sheet.getByTestId("workspace-menu-item-workspace-two"),
  ).toBeVisible();
  await expect(sheet).not.toContainText("⌘1");
  await expect(sheet).not.toContainText("⌘2");
  await expect(sheet.getByTestId("workspace-leave-action")).toHaveCount(0);
  await capture(page, "WSMobileMenuDark");
});
