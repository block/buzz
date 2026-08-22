import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SIDEBAR_WIDTH_STORAGE_KEY = "buzz-sidebar-width";
const COMMUNITY_ONBOARDING_STORAGE_KEY =
  "buzz-community-onboarding-transaction.v1";
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

async function openAddCommunityDialog(page: Page) {
  await page.getByTestId("sidebar-profile-avatar-button").click();
  await page.getByTestId("community-switcher").click();
  await page.getByRole("menuitem", { name: "Add a community" }).click();
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

test("sidebar rows separate hover, selected, and reorder states", async ({
  page,
}) => {
  await loadTheme(page, "github-light");
  await page.getByTestId("channel-general").click();

  const selectedRow = page.getByTestId("channel-general");
  const hoverRow = page.getByTestId("channel-random");

  await page.mouse.move(600, 100);
  const establishedActiveBackground = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--sidebar-active))";
    document.body.append(probe);
    const background = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return background;
  });
  await expect(selectedRow).toHaveCSS(
    "background-color",
    establishedActiveBackground,
  );
  // The spacing and motion experiment must preserve the production selected
  // row typography.
  await expect(selectedRow).toHaveCSS("font-weight", "400");

  const rowGap = await page.evaluate(() => {
    const selected = document.querySelector<HTMLElement>(
      '[data-testid="channel-general"]',
    );
    const following = document.querySelector<HTMLElement>(
      '[data-testid="channel-random"]',
    );
    if (!selected || !following) return null;
    const selectedBox = selected.getBoundingClientRect();
    const followingBox = following.getBoundingClientRect();
    return followingBox.top - selectedBox.bottom;
  });
  expect(rowGap).toBe(4);

  const establishedHoverBackground = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--sidebar-accent))";
    document.body.append(probe);
    const background = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return background;
  });
  await hoverRow.hover();
  await expect(hoverRow).toHaveCSS(
    "background-color",
    establishedHoverBackground,
  );

  const activeForegroundTokens = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    const sidebar = document.querySelector<HTMLElement>(
      '[data-testid="app-sidebar"]',
    );
    if (!sidebar) return null;
    return {
      root: root.getPropertyValue("--sidebar-active-foreground").trim(),
      sidebar: getComputedStyle(sidebar)
        .getPropertyValue("--sidebar-active-foreground")
        .trim(),
    };
  });
  expect(activeForegroundTokens).not.toBeNull();
  expect(activeForegroundTokens?.sidebar).toBe(activeForegroundTokens?.root);

  const selectedBox = await selectedRow.boundingBox();
  expect(selectedBox).not.toBeNull();
  if (!selectedBox) return;

  const draggableRow = selectedRow.locator("..");

  await page.mouse.move(
    selectedBox.x + selectedBox.width / 2,
    selectedBox.y + selectedBox.height / 2,
  );
  await page.mouse.down();
  await expect(selectedRow).toHaveCSS("transform", "none");
  await expect(draggableRow).not.toHaveAttribute("data-sidebar-drag-state");

  // Small pointer motion still counts as an ordinary click. The row only
  // scales once dnd-kit's 6px reorder threshold has been crossed.
  await page.mouse.move(
    selectedBox.x + selectedBox.width / 2,
    selectedBox.y + selectedBox.height / 2 + 3,
    { steps: 3 },
  );
  await expect(draggableRow).not.toHaveAttribute("data-sidebar-drag-state");
  await expect(draggableRow).toHaveCSS("transform", "none");

  await page.mouse.move(
    selectedBox.x + selectedBox.width / 2,
    selectedBox.y + selectedBox.height / 2 + 8,
    { steps: 5 },
  );
  await expect(draggableRow).toHaveAttribute(
    "data-sidebar-drag-state",
    "dragging",
  );
  await expect(page.getByTestId("sidebar-channel-drag-overlay")).toBeVisible();
  await expect
    .poll(() =>
      draggableRow.evaluate((element) => getComputedStyle(element).transform),
    )
    .toBe("matrix(0.985, 0, 0, 0.985, 0, 0)");
  await page.mouse.up();
  await expect(draggableRow).not.toHaveAttribute("data-sidebar-drag-state");
  await expect(
    page.getByTestId("sidebar-channel-drag-overlay"),
  ).not.toBeVisible();
  await expect
    .poll(() =>
      draggableRow.evaluate((element) => getComputedStyle(element).transform),
    )
    .toBe("none");
});

test("add community starts with create and join choices", async ({ page }) => {
  await installMockBridge(page, {});
  await page.goto("/");

  await openAddCommunityDialog(page);

  await expect(
    page.getByRole("heading", { name: "Add community" }),
  ).toBeVisible();
  await expect(page.getByTestId("add-community-create")).toContainText(
    "Create a new community",
  );
  await expect(page.getByTestId("add-community-join")).toContainText(
    "Join an existing community",
  );
  await expect(page.getByLabel("API Token")).toHaveCount(0);
  await expect(page.getByLabel("Repos Directory")).toHaveCount(0);

  await page.getByTestId("add-community-join").click();
  await expect(
    page.getByRole("heading", { name: "Join an existing community" }),
  ).toBeVisible();
  await expect(page.getByLabel("Community URL or invite link")).toBeVisible();
  await page.getByTestId("add-community-back").click();

  await page.getByTestId("add-community-create").click();
  await expect(
    page.getByRole("heading", { name: "Create a new community" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Continue to Builderlab" }).click();
  await page.getByRole("button", { name: "Connect and continue" }).click();
  await expect(page.getByLabel("Community address")).toBeVisible();
});

test("automatically shows community join requirements near the community URL", async ({
  page,
}) => {
  await installMockBridge(page, {
    applyCommunityDelayMs: 1_000,
    joinPolicy: {
      terms_markdown: "# Terms",
      privacy_markdown: "# Privacy",
      age_attestation_required: true,
      version: "policy-v1",
    },
  });
  await page.goto("/");

  await openAddCommunityDialog(page);
  await page.getByTestId("add-community-join").click();
  await page
    .getByLabel("Community URL or invite link")
    .fill("https://policy.example.com");

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

  const joinCommunityButton = page.getByTestId("invite-redeem-submit");
  await expect(joinCommunityButton).toBeDisabled();
  await ageConfirmation.check();
  await expect(ageConfirmation.locator("svg path")).toBeVisible();
  await expect(joinCommunityButton).toBeDisabled();
  await agreementConfirmation.check();
  await expect(joinCommunityButton).toBeEnabled();
  await expect(joinCommunityButton).toHaveText("Accept and join");

  const consentBox = await agreementConfirmation.boundingBox();
  const communityInput = await page
    .getByLabel("Community URL or invite link")
    .boundingBox();
  const joinButtonBox = await joinCommunityButton.boundingBox();
  expect(consentBox?.y).toBeGreaterThan(communityInput?.y ?? Number.MAX_VALUE);
  expect(consentBox?.y).toBeLessThan(joinButtonBox?.y ?? 0);

  await joinCommunityButton.click();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        COMMUNITY_ONBOARDING_STORAGE_KEY,
      ),
    )
    .toContain('"relayUrl":"wss://policy.example.com"');
});

test("joins a community URL without an API token field", async ({ page }) => {
  await installMockBridge(page, { applyCommunityDelayMs: 1_000 });
  await page.goto("/");

  await openAddCommunityDialog(page);
  await page.getByTestId("add-community-join").click();

  await page
    .getByLabel("Community URL or invite link")
    .fill("token.example.com");
  await expect(page.getByLabel("API token")).toHaveCount(0);
  await expect(page.getByTestId("community-api-token-reveal")).toHaveCount(0);
  await page.getByTestId("invite-redeem-submit").click();

  await expect
    .poll(() =>
      page.evaluate((key) => {
        const raw = window.localStorage.getItem(key);
        if (!raw) return null;
        return JSON.parse(raw) as { relayUrl?: string };
      }, COMMUNITY_ONBOARDING_STORAGE_KEY),
    )
    .toMatchObject({ relayUrl: "wss://token.example.com" });
});

test("hides Invites settings on open relays", async ({ page }) => {
  await page.goto("/");
  await openSettings(page);

  await expect(page.getByTestId("settings-nav-community-members")).toHaveCount(
    0,
  );
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

test("keeps only search pinned while primary navigation scrolls", async ({
  page,
}) => {
  await loadTheme(page, "github-light");

  const search = page.getByTestId("open-search");
  const primaryMenu = page.getByTestId("sidebar-primary-menu");
  const sidebarScroller = page.locator(".buzz-sidebar-scrollbar");
  const [initialSearchBox, initialMenuBox] = await Promise.all([
    search.boundingBox(),
    primaryMenu.boundingBox(),
  ]);
  expect(initialSearchBox).not.toBeNull();
  expect(initialMenuBox).not.toBeNull();

  const scrollTop = await sidebarScroller.evaluate((element) => {
    element.scrollTop = Math.min(
      120,
      Math.max(0, element.scrollHeight - element.clientHeight),
    );
    return element.scrollTop;
  });
  expect(scrollTop).toBeGreaterThan(0);
  await expect
    .poll(() =>
      sidebarScroller.evaluate((element) => Math.round(element.scrollTop)),
    )
    .toBe(Math.round(scrollTop));

  const [scrolledSearchBox, scrolledMenuBox] = await Promise.all([
    search.boundingBox(),
    primaryMenu.boundingBox(),
  ]);
  expect(scrolledSearchBox).not.toBeNull();
  expect(scrolledMenuBox).not.toBeNull();
  expect(
    Math.abs((scrolledSearchBox?.y ?? 0) - (initialSearchBox?.y ?? 0)),
  ).toBeLessThanOrEqual(1);
  expect(scrolledMenuBox?.y ?? 0).toBeLessThan(initialMenuBox?.y ?? 0);
});

test("scales the sidebar backward while its chrome closes", async ({
  page,
}) => {
  await page.goto("/");

  const sidebar = page.getByTestId("app-sidebar");
  const sidebarSurface = sidebar.locator("[data-sidebar-transition-content]");
  await expect(sidebarSurface).toHaveCSS("opacity", "1");
  await expect(sidebarSurface).toHaveCSS("scale", "none");

  await page.getByRole("button", { name: "Toggle Sidebar" }).click();

  await expect(sidebarSurface).toHaveCSS("opacity", "0");
  await expect(sidebar).toHaveCSS("pointer-events", "none");
  await expect(sidebar).toHaveCSS("overflow", "visible");
  await expect(sidebar.locator(':scope > [data-sidebar="sidebar"]')).toHaveCSS(
    "background-color",
    await sidebarSurface.evaluate((element) => {
      const sidebarElement = element.closest('[data-sidebar="sidebar"]');
      if (!(sidebarElement instanceof HTMLElement)) return "";
      return getComputedStyle(sidebarElement).backgroundColor;
    }),
  );
  await expect(sidebarSurface).toHaveCSS("scale", "0.95");
  await expect(sidebarSurface).toHaveCSS("translate", "24px");
  const transformOrigin = await sidebarSurface.evaluate(
    (element) => getComputedStyle(element).transformOrigin,
  );
  const [originX, originY] = transformOrigin.split(" ").map(Number.parseFloat);
  const surfaceWidth = await sidebarSurface.evaluate(
    (element) => element.clientWidth,
  );
  expect(Math.abs(originX - surfaceWidth / 2)).toBeLessThan(0.5);
  expect(originY).toBe(0);
  await expect(sidebarSurface).toHaveCSS(
    "transition-property",
    "opacity, scale, translate",
  );
  await expect(sidebarSurface).toHaveCSS("transition-duration", "0.2s");
  await expect(sidebarSurface).toHaveCSS(
    "transition-timing-function",
    "linear",
  );

  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await expect(sidebarSurface).toHaveCSS("opacity", "1");
  await expect(sidebar).toHaveCSS("pointer-events", "auto");
  await expect(sidebarSurface).toHaveCSS("scale", "none");
});

test("disables the sidebar collapse transition for reduced motion", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");

  const sidebarSurface = page
    .getByTestId("app-sidebar")
    .locator("[data-sidebar-transition-content]");
  await expect(sidebarSurface).toHaveCSS("transition-duration", "0s");

  await page.getByRole("button", { name: "Toggle Sidebar" }).click();

  await expect(sidebarSurface).toHaveCSS("opacity", "0");
  await expect(sidebarSurface).toHaveCSS("scale", "0.95");
  await expect(sidebarSurface).toHaveCSS("translate", "24px");
  await expect(sidebarSurface).toHaveCSS("transition-duration", "0s");
});

test("sidebar rail resizes without toggling the sidebar", async ({ page }) => {
  await page.goto("/");
  const rail = page.getByRole("button", { name: "Resize sidebar" });
  await rail.click();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await expect(rail).toBeHidden();
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
