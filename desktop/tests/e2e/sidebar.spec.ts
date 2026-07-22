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

async function loadTheme(page: Page, theme: string) {
  await page.addInitScript((selectedTheme) => {
    window.localStorage.setItem("buzz-theme", selectedTheme);
  }, theme);
  await installMockBridge(page);
  await page.goto("/");
}

// Regression guard for the "Leave channel" lockup: with two bundled copies of
// @radix-ui/react-dismissable-layer, opening a modal AlertDialog from a modal
// Radix ContextMenu left `pointer-events: none` stuck on <body> after the
// dialog closed, freezing the whole app. Fixed by the pnpm override in
// pnpm-workspace.yaml deduplicating the layer. This asserts the app is still
// interactive.
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

test("automatically shows relay join requirements near the relay URL", async ({
  page,
}) => {
  await page.route(
    "https://policy.example.com/api/join-policy",
    async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          policy: {
            terms_markdown: "# Terms",
            privacy_markdown: "# Privacy",
            age_attestation_required: true,
            version: "policy-v1",
          },
        }),
      });
    },
  );
  await page.goto("/");

  await page.getByTestId("sidebar-profile-card").click();
  await page.getByText("Add Community", { exact: true }).click();
  await page.getByLabel("Relay URL").fill("wss://policy.example.com");

  const ageConfirmation = page.getByLabel("I am 18 years of age or older.");
  const agreementConfirmation = page.getByLabel(
    "I agree to the Buzz Terms of Service and Privacy Policy.",
  );
  await expect(ageConfirmation).toBeVisible();
  await expect(agreementConfirmation).toBeVisible();
  await expect(
    page.getByText("Review this relay's join policy below."),
  ).toHaveCount(0);
  await expect(page.getByText(/By continuing, you agree/)).toHaveCount(0);

  const addCommunityButton = page.getByRole("button", {
    name: "Add Community",
  });
  await expect(addCommunityButton).toBeDisabled();
  await ageConfirmation.check();
  await expect(ageConfirmation.locator("svg path")).toBeVisible();
  await expect(addCommunityButton).toBeDisabled();
  await agreementConfirmation.check();
  await expect(addCommunityButton).toBeEnabled();

  const consentBox = await agreementConfirmation.boundingBox();
  const reposInput = await page.locator("#ws-repos-dir").boundingBox();
  const addButtonBox = await addCommunityButton.boundingBox();
  expect(consentBox?.y).toBeGreaterThan(reposInput?.y ?? Number.MAX_VALUE);
  expect(consentBox?.y).toBeLessThan(addButtonBox?.y ?? 0);
});

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

test("channel context menu only shows owner actions to the owner", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Archive channel" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Delete channel" }),
  ).toBeVisible();
  const lifecycleOrder = (
    await page.getByRole("menuitem").allTextContents()
  ).filter((label) =>
    ["Leave channel", "Archive channel", "Delete channel"].includes(label),
  );
  expect(lifecycleOrder).toEqual([
    "Leave channel",
    "Archive channel",
    "Delete channel",
  ]);
  await page.keyboard.press("Escape");

  await page.getByTestId("channel-random").click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Leave channel" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Loading channel actions..." }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: "Archive channel" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: "Delete channel" }),
  ).toHaveCount(0);
});

test("channel context menu explains when owner actions are loading", async ({
  page,
}) => {
  await installMockBridge(page, { channelMembersReadDelayMs: 500 });
  await page.goto("/");

  await page.getByTestId("channel-general").click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Loading channel actions..." }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Archive channel" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Loading channel actions..." }),
  ).toHaveCount(0);
});

test("channel owner can archive from the context menu", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Archive channel" }).click();

  await expect(page.getByTestId("stream-list")).not.toContainText("general");
});

test("channel owner can delete from the context menu", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  await page.getByTestId("channel-general").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Delete channel" }).click();
  await expect(
    page.getByTestId("channel-delete-confirmation-dialog"),
  ).toBeVisible();
  await page.getByTestId("channel-delete-confirm").click();

  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await expect(page.getByTestId("stream-list")).not.toContainText("general");
});

for (const theme of ["buzz", "github-light", "catppuccin-mocha"]) {
  test(`uses the continuous sidebar surface in ${theme}`, async ({ page }) => {
    await loadTheme(page, theme);

    const pinnedHeader = page.getByTestId("sidebar-pinned-header");
    const footer = page.locator(
      '[data-testid="app-sidebar"] [data-sidebar="footer"]',
    );
    const channelContent = page.getByTestId("sidebar-channel-content");
    await expect(pinnedHeader).toBeVisible();
    await expect(footer).toBeVisible();
    await expect(channelContent).toBeVisible();

    const chromeStyles = await page.evaluate(() => {
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
      const footerStyle = getComputedStyle(footerElement);
      const footerBefore = getComputedStyle(footerElement, "::before");
      const channelBefore = getComputedStyle(channelElement, "::before");
      const channelAfter = getComputedStyle(channelElement, "::after");

      return {
        channelAfterBackground: channelAfter.backgroundImage,
        channelBeforeBackground: channelBefore.backgroundImage,
        footerBackground: footerStyle.backgroundImage,
        footerBackgroundColor: footerStyle.backgroundColor,
        footerBeforeBackground: footerBefore.backgroundImage,
        footerBeforeContent: footerBefore.content,
        footerBoxShadow: footerStyle.boxShadow,
        footerIsolation: footerStyle.isolation,
        footerMarginTop: Number.parseFloat(footerStyle.marginTop),
        footerZIndex: footerStyle.zIndex,
        headerBackground: headerStyle.backgroundImage,
        headerBackgroundColor: headerStyle.backgroundColor,
        headerBeforeBackground: headerBefore.backgroundImage,
        headerBeforeContent: headerBefore.content,
        headerBoxShadow: headerStyle.boxShadow,
        headerIsolation: headerStyle.isolation,
        headerMarginBottom: Number.parseFloat(headerStyle.marginBottom),
        headerZIndex: headerStyle.zIndex,
      };
    });

    expect(chromeStyles.headerBackground).toBe("none");
    expect(chromeStyles.headerBackgroundColor).toBe("rgba(0, 0, 0, 0)");
    expect(chromeStyles.headerBeforeBackground).toBe("none");
    expect(chromeStyles.headerBeforeContent).toBe("none");
    expect(chromeStyles.headerBoxShadow).toBe("none");
    expect(chromeStyles.headerIsolation).toBe("auto");
    expect(chromeStyles.headerMarginBottom).toBe(0);
    expect(chromeStyles.headerZIndex).toBe("auto");
    expect(chromeStyles.footerBackground).toBe("none");
    expect(chromeStyles.footerBackgroundColor).toBe("rgba(0, 0, 0, 0)");
    expect(chromeStyles.footerBeforeBackground).toBe("none");
    expect(chromeStyles.footerBeforeContent).toBe("none");
    expect(chromeStyles.footerBoxShadow).toBe("none");
    expect(chromeStyles.footerIsolation).toBe("auto");
    expect(chromeStyles.footerMarginTop).toBe(0);
    expect(chromeStyles.footerZIndex).toBe("auto");
    expect(chromeStyles.channelBeforeBackground).toBe("none");
    expect(chromeStyles.channelAfterBackground).toBe("none");
  });
}

test("aligns the sidebar search with the channel title outside the Buzz theme", async ({
  page,
}) => {
  await loadTheme(page, "github-light");
  await page.getByTestId("channel-general").click();

  const root = page.locator("html");
  const search = page.getByTestId("open-search");
  const channelTitle = page.getByTestId("chat-title");
  await expect(root).not.toHaveAttribute("data-buzz-sidebar", "");
  await expect(search).toBeVisible();
  await expect(channelTitle).toHaveText("general");

  const [searchBox, channelTitleBox] = await Promise.all([
    search.boundingBox(),
    channelTitle.boundingBox(),
  ]);
  expect(searchBox).not.toBeNull();
  expect(channelTitleBox).not.toBeNull();

  if (!searchBox || !channelTitleBox) return;

  const searchCenter = searchBox.y + searchBox.height / 2;
  const channelTitleCenter = channelTitleBox.y + channelTitleBox.height / 2;
  expect(Math.abs(searchCenter - channelTitleCenter)).toBeLessThanOrEqual(2);
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
    "Update downloaded. Click to apply.",
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const commands =
          (
            window as Window & {
              __BUZZ_E2E_COMMANDS__?: string[];
            }
          ).__BUZZ_E2E_COMMANDS__ ?? [];
        return (
          commands.includes("plugin:updater|install") ||
          commands.includes("plugin:process|restart")
        );
      }),
    )
    .toBe(false);

  await page.getByTestId("settings-back-to-app").click();

  const updateCard = page.getByTestId("sidebar-update-card");
  await expect(updateCard).toBeVisible();
  await expect(updateCard).toContainText("Ready to update!");
  await expect(updateCard).toContainText("Click to update");
  await expect(page.getByTestId("sidebar-update-now")).toBeVisible();
  await page.getByTestId("sidebar-update-now").click();
  await expect(updateCard).toContainText("Updating");
  await expect(page.getByTestId("sidebar-update-now")).toBeDisabled();

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
    .toEqual(
      expect.arrayContaining([
        "plugin:updater|download",
        "plugin:updater|install",
        "plugin:process|restart",
      ]),
    );

  const commands = await page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMANDS__?: string[];
        }
      ).__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands.indexOf("plugin:updater|download")).toBeLessThan(
    commands.indexOf("plugin:updater|install"),
  );
  expect(commands.indexOf("plugin:updater|install")).toBeLessThan(
    commands.indexOf("plugin:process|restart"),
  );
});

// Regression test for the sidebar card not reflecting an install started from
// another surface (follow-up to #1820). The header UpdateIndicator and the
// sidebar compact card both render in the "ready" state; starting the install
// from the header must flip the sidebar card's copy too, not just the header's.
test("reflects an install started from the header update button on the sidebar card", async ({
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
    "Update downloaded. Click to apply.",
  );
  await page.getByTestId("settings-back-to-app").click();

  await page.getByTestId("channel-general").click();

  const updateCard = page.getByTestId("sidebar-update-card");
  await expect(updateCard).toBeVisible();
  await expect(updateCard).toContainText("Click to update");

  await page
    .getByTestId("chat-header")
    .getByRole("button", { name: "Update now" })
    .click();

  await expect(updateCard).toContainText("Updating");
  await expect(page.getByTestId("sidebar-update-now")).toBeDisabled();
});

// Regression test for the Linux .deb auto-update guard (PR #1535).
// When auto-update is not supported (e.g. Linux .deb install), the update
// check must surface a "manual-required" card with a GitHub link and
// AppImage hint, and must NEVER invoke the in-app download or install commands.
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

  // In-app download and install must NEVER have been called.
  const commands = await page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMANDS__?: string[];
        }
      ).__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).not.toContain("plugin:updater|download");
  expect(commands).not.toContain("plugin:updater|install");
});
