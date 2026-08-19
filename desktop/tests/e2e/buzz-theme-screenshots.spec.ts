import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/buzz-theme";
const THEME_STORAGE_KEY = "buzz-theme";
const GLASS_BACKGROUND_STORAGE_KEY = "buzz-glass-background";
const GLASS_OPACITY_STORAGE_KEY = "buzz-glass-opacity";
const PROMINENT_ACTIVE_TAB_STORAGE_KEY = "buzz-prominent-active-tab";
const CONVERSATION_DENSITY_STORAGE_KEY = "buzz.appearance.conversationDensity";
const FONT_SIZE_STORAGE_KEY = "buzz.appearance.fontSize";
const MOCK_PUBKEY = "deadbeef".repeat(8);
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

/**
 * Seed the active theme into localStorage BEFORE the mock bridge installs so
 * ThemeProvider reads it on first mount (init scripts run in registration
 * order; React reads state on mount, which the bridge triggers).
 */
async function seedTheme(page: Page, theme: string) {
  await page.addInitScript(
    ({ key, value }) => {
      window.localStorage.setItem(key, value);
    },
    { key: THEME_STORAGE_KEY, value: theme },
  );
}

async function seedIconChannelSection(page: Page) {
  await page.addInitScript(
    ({ channelId, pubkey }) => {
      window.localStorage.setItem(
        `buzz-channel-sections.v1:${pubkey}`,
        JSON.stringify({
          version: 1,
          sections: [
            {
              id: "alignment-section",
              name: "Team channels",
              icon: "📌",
              order: 0,
            },
          ],
          assignments: { [channelId]: "alignment-section" },
        }),
      );
    },
    { channelId: GENERAL_CHANNEL_ID, pubkey: MOCK_PUBKEY },
  );
}

async function openChannel(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
}

async function expectBuzzSidebarPalette(page: Page, mode: "light" | "dark") {
  const mutedColor =
    mode === "light" ? "rgba(0, 0, 0, 0.4)" : "rgba(255, 255, 255, 0.4)";
  const searchSurface =
    mode === "light" ? "rgba(0, 0, 0, 0.04)" : "rgba(255, 255, 255, 0.04)";
  const rowHoverSurface =
    mode === "light" ? "rgba(0, 0, 0, 0.04)" : "rgba(255, 255, 255, 0.04)";
  const activeSurface =
    mode === "light" ? "rgba(0, 0, 0, 0.07)" : "rgba(255, 255, 255, 0.16)";
  const chromeColor =
    mode === "light" ? "rgba(0, 0, 0, 0.5)" : "rgba(255, 255, 255, 0.5)";
  const search = page.getByTestId("open-search");
  const pinnedHeader = page.getByTestId("sidebar-pinned-header");
  const sidebarScroller = page.locator(".buzz-sidebar-scrollbar");
  const scrollContent = page.getByTestId("sidebar-scroll-content");
  const primaryMenu = page.getByTestId("sidebar-primary-menu");
  const sectionLabel = page
    .locator('[data-sidebar="group-label"]')
    .filter({ hasText: "Channels" })
    .first();

  await expect(sectionLabel).toHaveCSS("color", mutedColor);
  await expect(search).toHaveCSS("background-color", searchSurface);
  await expect(search.locator("svg").first()).toHaveCSS("color", mutedColor);
  await expect(search.locator("span").first()).toHaveCSS("color", mutedColor);
  await expect(pinnedHeader).toHaveCSS("padding-bottom", "8px");
  await expect(pinnedHeader).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(pinnedHeader).toHaveCSS("margin-left", "3px");
  await expect(pinnedHeader).toHaveCSS("margin-right", "3px");
  await expect(pinnedHeader).toHaveCSS("padding-right", "8px");
  await expect(sidebarScroller).toHaveCSS("padding-left", "0px");
  await expect(sidebarScroller).toHaveCSS("padding-right", "0px");
  await expect(scrollContent).toHaveCSS("padding-left", "3px");
  await expect(scrollContent).toHaveCSS("padding-right", "3px");
  const pinnedSpacerColor = await pinnedHeader.evaluate(
    (element) => getComputedStyle(element, "::before").backgroundColor,
  );
  expect(pinnedSpacerColor).toBe("rgba(0, 0, 0, 0)");
  await expect(sidebarScroller.getByTestId("open-agents-view")).toBeVisible();
  const searchBox = await search.boundingBox();
  const pinnedHeaderBox = await pinnedHeader.boundingBox();
  const primaryMenuBox = await primaryMenu.boundingBox();
  const primaryRowBox = await page
    .getByTestId("open-agents-view")
    .boundingBox();
  const activeRowBox = await page.getByTestId("channel-general").boundingBox();
  const hoverRowBox = await page.getByTestId("channel-random").boundingBox();
  const scrollContentBox = await scrollContent.evaluate((element) => {
    const box = element.getBoundingClientRect();
    return { left: box.left, right: box.right };
  });
  expect(searchBox).not.toBeNull();
  expect(pinnedHeaderBox).not.toBeNull();
  expect(primaryMenuBox).not.toBeNull();
  expect(primaryRowBox).not.toBeNull();
  expect(activeRowBox).not.toBeNull();
  expect(hoverRowBox).not.toBeNull();
  if (
    !searchBox ||
    !pinnedHeaderBox ||
    !primaryMenuBox ||
    !primaryRowBox ||
    !activeRowBox ||
    !hoverRowBox
  ) {
    throw new Error("Sidebar search or primary navigation geometry is missing");
  }
  expect(primaryMenuBox.y - (searchBox.y + searchBox.height)).toBe(8);
  expect(
    pinnedHeaderBox.y +
      pinnedHeaderBox.height -
      (searchBox.y + searchBox.height),
  ).toBe(8);
  expect(primaryMenuBox.y - (pinnedHeaderBox.y + pinnedHeaderBox.height)).toBe(
    0,
  );
  for (const rowBox of [primaryRowBox, activeRowBox, hoverRowBox]) {
    expect(Math.abs(rowBox.x - searchBox.x)).toBeLessThanOrEqual(0.5);
    // Linux CI reserves a classic scrollbar gutter while macOS uses an
    // overlay scrollbar. Compare each row to its usable scroll area so the
    // alignment check remains platform-independent.
    const rowLeftSpacing = rowBox.x - scrollContentBox.left;
    const rowRightSpacing = scrollContentBox.right - (rowBox.x + rowBox.width);
    expect(Math.abs(rowLeftSpacing - rowRightSpacing)).toBeLessThanOrEqual(0.5);
  }
  await expect(page.locator("[data-buzz-sidebar-secondary]").first()).toHaveCSS(
    "color",
    mutedColor,
  );
  await expect(page.locator('[data-sidebar="trigger"]')).toHaveCSS(
    "color",
    chromeColor,
  );
  await expect(page.getByTestId("global-back")).toHaveCSS("color", chromeColor);
  await expect(page.getByTestId("global-forward")).toHaveCSS(
    "color",
    chromeColor,
  );
  await expect(page.getByTestId("channel-general")).not.toHaveCSS(
    "color",
    mutedColor,
  );
  await expect(page.getByTestId("channel-general")).toHaveCSS(
    "font-weight",
    "400",
  );
  await expect(
    page.getByTestId("channel-general").locator("[data-sidebar-row-label]"),
  ).toHaveCSS("opacity", "1");
  await page.mouse.move(600, 100);
  await expect(page.getByTestId("channel-general")).toHaveCSS(
    "background-color",
    activeSurface,
  );
  const hoverChannel = page.getByTestId("channel-random");
  await hoverChannel.hover();
  await expect(hoverChannel).toHaveCSS("background-color", rowHoverSurface);

  const firstDmItem = page
    .getByTestId("dm-list")
    .locator('[data-sidebar="menu-item"]')
    .first();
  const firstDmButton = firstDmItem.locator('[data-sidebar="menu-button"]');
  const firstDmLabel = firstDmButton.locator("[data-sidebar-row-label]");
  const closeDmButton = firstDmItem.getByRole("button", {
    name: "Close direct message",
  });
  const hoverChannelLabel = hoverChannel.locator("[data-sidebar-row-label]");
  const hoverChannelIcon = hoverChannel.locator("svg").first();
  const agentsButton = page.getByTestId("open-agents-view");
  const agentsLabel = agentsButton.locator('[data-sidebar="menu-label"]');
  const agentsIcon = agentsButton.locator("svg").first();
  const sidebarForeground = await page
    .getByTestId("app-sidebar")
    .evaluate((element) => getComputedStyle(element).color);
  await expect(hoverChannel).toHaveCSS("color", sidebarForeground);
  await expect(firstDmButton).toHaveCSS("color", sidebarForeground);
  await expect(agentsButton).toHaveCSS("color", sidebarForeground);
  await expect(hoverChannelLabel).toHaveCSS("opacity", "0.8");
  await expect(hoverChannelIcon).toHaveCSS("opacity", "0.8");
  await expect(firstDmButton).toHaveCSS("opacity", "1");
  await expect(firstDmLabel).toHaveCSS("opacity", "0.8");
  await expect(agentsLabel).toHaveCSS("opacity", "0.8");
  await expect(agentsIcon).toHaveCSS("opacity", "0.8");
  await firstDmItem.hover();
  await expect(closeDmButton).toBeVisible();
  await closeDmButton.hover();
  await expect(firstDmButton).toHaveCSS("background-color", rowHoverSurface);

  const scrollbarThumbColor = await sidebarScroller.evaluate(
    (element) =>
      getComputedStyle(element, "::-webkit-scrollbar-thumb").backgroundColor,
  );
  expect(scrollbarThumbColor).toBe(searchSurface);
}

async function expectIconlessSectionTitleAligned(
  page: Page,
  listTestId: "stream-list" | "dm-list",
) {
  const titleBox = await page
    .getByTestId(`${listTestId}-section-label`)
    .locator("[data-sidebar-section-title]")
    .boundingBox();
  const firstRowIconX = await page
    .getByTestId(listTestId)
    .locator('[data-sidebar="menu-button"]')
    .first()
    .evaluate((element) => {
      const box = element.getBoundingClientRect();
      const paddingLeft = Number.parseFloat(
        getComputedStyle(element).paddingLeft,
      );
      return box.x + paddingLeft;
    });

  expect(titleBox).not.toBeNull();
  if (!titleBox) {
    throw new Error(`Sidebar section ${listTestId} is missing label geometry`);
  }
  expect(Math.abs(titleBox.x - firstRowIconX)).toBeLessThanOrEqual(0.5);
}

async function expectBuzzContentShadow(page: Page, mode: "light" | "dark") {
  const effects = await page.evaluate(() => {
    const shell = document.querySelector(".buzz-huddle-shell");
    const content = document.querySelector("[data-buzz-content-surface]");
    const shadowViewport = document.querySelector(
      "[data-buzz-shadow-viewport]",
    );
    return {
      appStroke: shell ? getComputedStyle(shell, "::before").boxShadow : "",
      contentShadow: content ? getComputedStyle(content).boxShadow : "",
      shadowViewportOverflow: shadowViewport
        ? getComputedStyle(shadowViewport).overflow
        : "",
    };
  });

  expect(effects.appStroke).toBe("none");
  if (mode === "light") {
    expect(effects.contentShadow).toContain("4px");
    expect(effects.contentShadow).toContain("rgba(0, 0, 0, 0.07)");
    expect(effects.shadowViewportOverflow).toBe("visible");
  } else {
    expect(effects.contentShadow).not.toContain("4px");
    expect(effects.contentShadow).not.toContain("rgba(255, 255, 255, 0.07)");
    expect(effects.shadowViewportOverflow).toBe("hidden");
  }
}

async function expectBuzzGradientPaint(
  page: Page,
  mode: "light" | "dark",
): Promise<string> {
  const paint = await page.evaluate(() => {
    const root = document.documentElement;
    const appSurface = document.querySelector(".buzz-huddle-app-surface");
    const lightLayer = document.querySelector('[data-buzz-gradient="light"]');
    const darkLayer = document.querySelector('[data-buzz-gradient="dark"]');
    const sidebarRoot = document.querySelector(
      '[data-testid="app-sidebar"], [data-testid="settings-sidebar"]',
    );
    const sidebarSurface =
      sidebarRoot?.querySelector('[data-sidebar="sidebar"]') ?? sidebarRoot;
    const appStyles = appSurface ? getComputedStyle(appSurface) : null;
    const lightStyles = lightLayer ? getComputedStyle(lightLayer) : null;
    const darkStyles = darkLayer ? getComputedStyle(darkLayer) : null;
    return {
      isDark: root.classList.contains("dark"),
      theme: root.getAttribute("data-buzz-theme"),
      surfaceImage: appStyles?.backgroundImage ?? "",
      lightImage: lightStyles?.backgroundImage ?? "",
      lightOpacity: lightStyles?.opacity ?? "",
      darkImage: darkStyles?.backgroundImage ?? "",
      darkOpacity: darkStyles?.opacity ?? "",
      sidebarImage: sidebarSurface
        ? getComputedStyle(sidebarSurface).backgroundImage
        : "",
    };
  });

  expect(paint.theme).toBe(mode === "light" ? "buzz" : "buzz-dark");
  expect(paint.isDark).toBe(mode === "dark");
  expect(paint.surfaceImage).toBe("none");
  expect(paint.lightImage).not.toBe("");
  expect(paint.lightImage).not.toBe("none");
  expect(paint.darkImage).not.toBe("");
  expect(paint.darkImage).not.toBe("none");
  expect(paint.lightImage).not.toBe(paint.darkImage);
  expect(paint.lightOpacity).toBe(mode === "light" ? "1" : "0");
  expect(paint.darkOpacity).toBe(mode === "dark" ? "1" : "0");
  expect(paint.sidebarImage).toBe("none");
  return mode === "light" ? paint.lightImage : paint.darkImage;
}

async function expectBuzzSettingsPalette(page: Page, mode: "light" | "dark") {
  const mutedColor =
    mode === "light" ? "rgba(0, 0, 0, 0.4)" : "rgba(255, 255, 255, 0.4)";
  const sidebar = page.getByTestId("settings-sidebar");
  const sectionLabel = sidebar
    .locator('[data-sidebar="group-label"]')
    .filter({ hasText: "Personal" });

  await expect(sectionLabel).toHaveCSS("color", mutedColor);
  await expect(page.getByTestId("settings-nav-profile")).not.toHaveCSS(
    "color",
    mutedColor,
  );

  await expectBuzzGradientPaint(page, mode);

  const version = page.getByTestId("settings-version");
  if ((await version.count()) > 0) {
    await expect(version).toHaveCSS("color", mutedColor);
  }
}

async function expectAppliedBuzzTheme(
  page: Page,
  themeName: "buzz" | "buzz-dark",
  storedTheme: "buzz" | "buzz-dark" = themeName,
) {
  const isDark = themeName === "buzz-dark";
  await expect
    .poll(() =>
      page.evaluate((storageKey) => {
        const root = document.documentElement;
        const styles = getComputedStyle(root);
        return {
          storedTheme: window.localStorage.getItem(storageKey),
          isDark: root.classList.contains("dark"),
          buzzTheme: root.getAttribute("data-buzz-theme"),
          gradientTop: styles.getPropertyValue("--buzz-gradient-top").trim(),
          gradientBottom: styles
            .getPropertyValue("--buzz-gradient-bottom")
            .trim(),
        };
      }, THEME_STORAGE_KEY),
    )
    .toEqual({
      storedTheme,
      isDark,
      buzzTheme: themeName,
      gradientTop: isDark ? "#4a4616" : "#e6e6b6",
      gradientBottom: isDark ? "#0a1423" : "#c4d0da",
    });
}

async function emitNativeThemeChange(page: Page, theme: "light" | "dark") {
  await page.evaluate(async (nextTheme) => {
    const tauriWindow = window as typeof window & {
      __TAURI_INTERNALS__?: {
        invoke?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const invoke = tauriWindow.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Mock Tauri invoke bridge is unavailable.");
    await invoke("plugin:event|emit", {
      event: "tauri://theme-changed",
      payload: nextTheme,
    });
  }, theme);
}

test("buzz light sidebar gradient", async ({ page }) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await openChannel(page);
  await expectBuzzGradientPaint(page, "light");
  await expectBuzzSidebarPalette(page, "light");
  await expectBuzzContentShadow(page, "light");
  await expectIconlessSectionTitleAligned(page, "stream-list");
  await expectIconlessSectionTitleAligned(page, "dm-list");
  await waitForAnimations(page);
  await page
    .getByTestId("app-sidebar")
    .screenshot({ path: `${SHOTS}/01-buzz-light-sidebar.png` });
});

test("buzz dark sidebar gradient", async ({ page }) => {
  await seedTheme(page, "buzz-dark");
  await installMockBridge(page);
  await openChannel(page);
  await expectBuzzGradientPaint(page, "dark");
  await expectBuzzSidebarPalette(page, "dark");
  await expectBuzzContentShadow(page, "dark");
  await expectIconlessSectionTitleAligned(page, "stream-list");
  await expectIconlessSectionTitleAligned(page, "dm-list");
  await expect(page.locator("[data-buzz-content-surface]")).toHaveCSS(
    "background-color",
    "rgb(26, 26, 26)",
  );
  await waitForAnimations(page);
  await page
    .getByTestId("app-sidebar")
    .screenshot({ path: `${SHOTS}/02-buzz-dark-sidebar.png` });
});

test("custom section icon and name align with channel columns", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await seedIconChannelSection(page);
  await installMockBridge(page);
  await openChannel(page);

  const sectionIconBox = await page
    .getByTestId("section-icon-alignment-section")
    .boundingBox();
  const sectionTitleBox = await page
    .getByTestId("section-title-alignment-section")
    .boundingBox();
  const channelButton = page.getByTestId("channel-general");
  const channelIconBox = await channelButton
    .locator("svg")
    .first()
    .boundingBox();
  const channelTitleBox = await channelButton
    .locator("[data-sidebar-row-label]")
    .boundingBox();

  expect(sectionIconBox).not.toBeNull();
  expect(sectionTitleBox).not.toBeNull();
  expect(channelIconBox).not.toBeNull();
  expect(channelTitleBox).not.toBeNull();
  if (
    !sectionIconBox ||
    !sectionTitleBox ||
    !channelIconBox ||
    !channelTitleBox
  ) {
    throw new Error("Custom section alignment geometry is missing");
  }
  expect(Math.abs(sectionIconBox.x - channelIconBox.x)).toBeLessThanOrEqual(
    0.5,
  );
  expect(Math.abs(sectionTitleBox.x - channelTitleBox.x)).toBeLessThanOrEqual(
    0.5,
  );
});

async function openAppearance(page: Page, mode: "system" | "light" | "dark") {
  // Settings renders at the AppShell level; open it via the profile card
  // button, then select the Appearance section.
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-appearance").click();
  const panel = page.getByTestId("settings-theme");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await page.getByTestId(`appearance-mode-${mode}`).click();
  await waitForAnimations(page);
  return panel;
}

test("appearance groups theme and preferences into labeled rows", async ({
  page,
}) => {
  await seedTheme(page, "github-light");
  await page.addInitScript((prominentActiveTabStorageKey) => {
    window.localStorage.setItem(prominentActiveTabStorageKey, "true");
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "MacIntel",
    });
  }, PROMINENT_ACTIVE_TAB_STORAGE_KEY);
  await installMockBridge(page);
  await openAppearance(page, "light");
  const themeCard = page.getByTestId("appearance-theme-card");
  const preferencesCard = page.getByTestId("appearance-preferences-card");

  await expect(
    themeCard.getByRole("heading", { name: "Theme", exact: true }),
  ).toBeVisible();
  await expect(
    preferencesCard.getByRole("heading", {
      name: "Preferences",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    themeCard.getByRole("group", { name: "Color mode" }),
  ).toBeVisible();
  await expect(themeCard.getByTestId("glass-background-toggle")).toBeVisible();
  await expect(
    themeCard.getByTestId("prominent-active-tab-toggle"),
  ).toHaveCount(0);
  await expect(
    preferencesCard.getByTestId("prominent-active-tab-toggle"),
  ).toHaveCount(0);
  await expect(
    preferencesCard.getByRole("group", { name: "Thread layout" }),
  ).toBeVisible();
  const themeStyleTrigger = themeCard.getByTestId("theme-style-trigger");
  const themeStyleOptions = themeCard.getByTestId("theme-style-options");
  const selectedThemePreview = themeCard.getByTestId(
    "theme-style-selected-preview",
  );
  await expect(themeStyleTrigger).toHaveAttribute(
    "aria-label",
    "Theme style, Github Light",
  );
  await expect(themeStyleTrigger).not.toContainText("Github Light");
  await expect(themeStyleTrigger).toHaveAttribute("aria-expanded", "false");
  await expect(themeStyleTrigger).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(themeStyleTrigger).toHaveCSS("border-width", "0px");
  await expect(selectedThemePreview).toBeVisible();
  const selectedThemePreviewSvg = selectedThemePreview.locator("svg");
  await expect(selectedThemePreviewSvg).toBeVisible();
  const selectedThemePreviewBox = await selectedThemePreview.boundingBox();
  const selectedThemePreviewSvgBox =
    await selectedThemePreviewSvg.boundingBox();
  expect(selectedThemePreviewBox?.width).toBe(168);
  expect(selectedThemePreviewBox?.height).toBe(112);
  expect(selectedThemePreviewSvgBox?.width).toBeGreaterThan(100);
  expect(selectedThemePreviewSvgBox?.height).toBeGreaterThan(70);
  await expect(themeStyleOptions).toHaveCount(0);

  await themeStyleTrigger.click();
  await expect(themeStyleTrigger).toHaveAttribute("aria-expanded", "true");
  await expect(themeStyleOptions).toBeVisible();
  await themeCard.getByTestId("theme-option-buzz").click();
  await expect(themeStyleTrigger).toHaveAttribute("aria-expanded", "true");
  await expect(themeStyleOptions).toBeVisible();
  await expect(
    themeCard.getByTestId("prominent-active-tab-toggle"),
  ).toBeChecked();
  const glassBeforeProminent = await themeCard.evaluate((card) => {
    const glass = card.querySelector('[data-testid="glass-background-toggle"]');
    const prominent = card.querySelector(
      '[data-testid="prominent-active-tab-row"]',
    );
    return Boolean(
      glass &&
        prominent &&
        glass.compareDocumentPosition(prominent) &
          Node.DOCUMENT_POSITION_FOLLOWING,
    );
  });
  expect(glassBeforeProminent).toBe(true);

  const colorModeLabelBox = await themeCard
    .getByTestId("appearance-color-mode-row")
    .locator("p")
    .first()
    .boundingBox();
  const colorModeControlBox = await themeCard
    .getByRole("group", { name: "Color mode" })
    .boundingBox();
  const glassLabelBox = await themeCard
    .getByText("Glass background", { exact: true })
    .boundingBox();
  const glassToggleBox = await themeCard
    .getByTestId("glass-background-toggle")
    .boundingBox();
  const colorModeIndicator = themeCard.getByTestId(
    "appearance-color-mode-indicator",
  );
  const lightModeButton = themeCard.getByTestId("appearance-mode-light");
  const initialIndicatorBox = await colorModeIndicator.boundingBox();
  const lightModeButtonBox = await lightModeButton.boundingBox();
  const themeCardBox = await themeCard.boundingBox();
  const preferencesCardBox = await preferencesCard.boundingBox();

  expect(colorModeLabelBox).not.toBeNull();
  expect(colorModeControlBox).not.toBeNull();
  expect(glassLabelBox).not.toBeNull();
  expect(glassToggleBox).not.toBeNull();
  expect(initialIndicatorBox).not.toBeNull();
  expect(lightModeButtonBox).not.toBeNull();
  expect(themeCardBox).not.toBeNull();
  expect(preferencesCardBox).not.toBeNull();
  if (
    !colorModeLabelBox ||
    !colorModeControlBox ||
    !glassLabelBox ||
    !glassToggleBox ||
    !initialIndicatorBox ||
    !lightModeButtonBox ||
    !themeCardBox ||
    !preferencesCardBox
  ) {
    throw new Error("Appearance card geometry is missing");
  }

  expect(colorModeLabelBox.x).toBeLessThan(colorModeControlBox.x);
  expect(glassLabelBox.x).toBeLessThan(glassToggleBox.x);
  expect(themeCardBox.y).toBeLessThan(preferencesCardBox.y);
  expect(
    Math.abs(
      initialIndicatorBox.x +
        initialIndicatorBox.width / 2 -
        (lightModeButtonBox.x + lightModeButtonBox.width / 2),
    ),
  ).toBeLessThanOrEqual(0.5);
  await expect(colorModeIndicator).toHaveCSS("transition-duration", "0.2s");

  await themeCard.getByTestId("appearance-mode-dark").click();
  await waitForAnimations(page);
  const movedIndicatorBox = await colorModeIndicator.boundingBox();
  const darkModeButtonBox = await themeCard
    .getByTestId("appearance-mode-dark")
    .boundingBox();
  expect(movedIndicatorBox).not.toBeNull();
  expect(darkModeButtonBox).not.toBeNull();
  if (!movedIndicatorBox || !darkModeButtonBox) {
    throw new Error("Color mode indicator geometry is missing");
  }
  expect(
    Math.abs(
      movedIndicatorBox.x +
        movedIndicatorBox.width / 2 -
        (darkModeButtonBox.x + darkModeButtonBox.width / 2),
    ),
  ).toBeLessThanOrEqual(0.5);
});

test("app font size and conversation density apply independently", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await openAppearance(page, "light");

  const root = page.locator("html");
  const densityControl = page.getByTestId("conversation-density-control");
  const compact = page.getByTestId("conversation-density-compact");
  const comfortable = page.getByTestId("conversation-density-comfortable");
  const spacious = page.getByTestId("conversation-density-spacious");
  const densityIndicator = page.getByTestId(
    "conversation-density-control-indicator",
  );
  const fontSizeControl = page.getByTestId("font-size-control");
  const smaller = page.getByTestId("font-size-smaller");
  const defaultSize = page.getByTestId("font-size-default");
  const larger = page.getByTestId("font-size-larger");
  const fontSizeIndicator = page.getByTestId("font-size-control-indicator");
  const preview = page.getByTestId("conversation-preview");
  const previewSurface = page.getByTestId("conversation-preview-surface");
  const previewContent = page.getByTestId("conversation-preview-content");
  const previewChip = preview.getByText("Preview");
  const firstPreviewMessage = previewSurface.locator("article").first();
  const previewMessage = preview.getByText(
    "The revised conversation layout is ready to review.",
  );
  const previewTimestamp = preview.getByText("9:41");
  const densityDescription = page
    .getByTestId("conversation-density-row")
    .locator("[data-settings-subcopy]");
  const fontSizeDescription = page
    .getByTestId("font-size-row")
    .locator("[data-settings-subcopy]");
  const readScale = () =>
    root.evaluate((element) => {
      const style = window.getComputedStyle(element);
      return {
        authorLineHeight: Number.parseFloat(
          style.getPropertyValue("--conversation-author-line-height"),
        ),
        bodyGap: Number.parseFloat(
          style.getPropertyValue("--conversation-body-gap"),
        ),
        fontSize: style.getPropertyValue("--conversation-message-font-size"),
        lineHeight: style.getPropertyValue(
          "--conversation-message-line-height",
        ),
        paragraphGap: Number.parseFloat(
          style.getPropertyValue("--conversation-paragraph-gap"),
        ),
        rowPadding: Number.parseFloat(
          style.getPropertyValue("--conversation-row-padding-block"),
        ),
        timestampFontSize: style.getPropertyValue(
          "--conversation-timestamp-font-size",
        ),
        timestampLineHeight: Number.parseFloat(
          style.getPropertyValue("--conversation-timestamp-line-height"),
        ),
      };
    });
  const readSettingsScale = () =>
    page.getByTestId("conversation-density-row").evaluate((element) => {
      const rowStyle = window.getComputedStyle(element);
      const label = element.querySelector("p");
      if (!label) throw new Error("Conversation density label is missing");
      const labelStyle = window.getComputedStyle(label);
      return {
        fontSize: labelStyle.fontSize,
        lineHeight: labelStyle.lineHeight,
        minHeight: rowStyle.minHeight,
        paddingBlock: rowStyle.paddingTop,
      };
    });
  const readPreviewTimestampScale = () =>
    previewTimestamp.evaluate((element) => {
      const style = window.getComputedStyle(element);
      return [style.fontSize, style.lineHeight];
    });
  const readSettingsChromeScale = () =>
    Promise.all([
      page
        .getByRole("heading", { name: "Appearance" })
        .evaluate((element) => window.getComputedStyle(element).fontSize),
      page
        .getByTestId("settings-nav-appearance")
        .evaluate((element) => window.getComputedStyle(element).fontSize),
      page
        .getByRole("heading", { name: "Preferences" })
        .evaluate((element) => window.getComputedStyle(element).fontSize),
    ]);

  await expect(
    page.getByRole("group", { name: "Conversation density" }),
  ).toBeVisible();
  await expect(page.getByRole("group", { name: "Font size" })).toBeVisible();
  await expect(densityControl).toHaveAccessibleName("Conversation density");
  await expect(fontSizeControl).toHaveAccessibleName("Font size");
  await expect(comfortable).toHaveText("Comfy");
  await expect(defaultSize).toHaveText("Default");
  await expect(preview).toContainText("Preview");
  await expect(preview).not.toContainText("Message #design");
  await expect(comfortable).toHaveAttribute("aria-pressed", "true");
  await expect(defaultSize).toHaveAttribute("aria-pressed", "true");
  await expect(densityDescription).toHaveText(
    "Spacing in conversations and Markdown content across Buzz",
  );
  await expect(fontSizeDescription).toHaveText(
    "Applies across conversations and interface text",
  );
  await expect.poll(readScale).toEqual({
    authorLineHeight: 16,
    bodyGap: 0.125,
    fontSize: "calc(16px * .875)",
    lineHeight: "calc(16px * 1.25)",
    paragraphGap: 0.5,
    rowPadding: 0.25,
    timestampFontSize: "calc(16px * .75)",
    timestampLineHeight: 16,
  });
  await expect
    .poll(() =>
      Promise.all([
        densityControl.evaluate(
          (element) => element.getBoundingClientRect().width,
        ),
        fontSizeControl.evaluate(
          (element) => element.getBoundingClientRect().width,
        ),
        page
          .getByTestId("appearance-color-mode-control")
          .evaluate((element) => element.getBoundingClientRect().width),
      ]),
    )
    .toEqual([288, 288, 240]);
  await expect
    .poll(() =>
      previewMessage.evaluate((element) => {
        const style = window.getComputedStyle(element);
        return [style.fontSize, style.lineHeight];
      }),
    )
    .toEqual(["14px", "20px"]);
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "14px",
    lineHeight: "20px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await expect.poll(readPreviewTimestampScale).toEqual(["12px", "16px"]);
  await expect.poll(readSettingsChromeScale).toEqual(["24px", "14px", "14px"]);
  await expect(densityIndicator).toHaveCSS("transition-duration", "0.2s");
  await expect(densityIndicator).toHaveCSS("transition-property", /transform/);
  await expect(fontSizeIndicator).toHaveCSS("transition-duration", "0.2s");
  await expect(fontSizeIndicator).toHaveCSS("transition-property", /transform/);
  await expect
    .poll(async () => {
      const [previewBackground, labelBackground, controlBackground] =
        await Promise.all([
          previewSurface.evaluate(
            (element) => window.getComputedStyle(element).backgroundColor,
          ),
          previewChip.evaluate(
            (element) => window.getComputedStyle(element).backgroundColor,
          ),
          densityControl.evaluate(
            (element) => window.getComputedStyle(element).backgroundColor,
          ),
        ]);
      return {
        labelIsAnnotation: labelBackground !== controlBackground,
        previewBackground,
      };
    })
    .toEqual({
      labelIsAnnotation: true,
      previewBackground: "rgba(0, 0, 0, 0)",
    });
  const previewSurfaceBox = await previewSurface.boundingBox();
  const previewChipBox = await previewChip.boundingBox();
  const firstPreviewMessageBox = await firstPreviewMessage.boundingBox();
  expect(previewSurfaceBox).not.toBeNull();
  expect(previewChipBox).not.toBeNull();
  expect(firstPreviewMessageBox).not.toBeNull();
  if (!previewSurfaceBox || !previewChipBox || !firstPreviewMessageBox) {
    throw new Error("Conversation preview geometry is missing");
  }
  const previewChipRightInset =
    previewSurfaceBox.x +
    previewSurfaceBox.width -
    (previewChipBox.x + previewChipBox.width);
  expect(previewChipRightInset).toBeGreaterThanOrEqual(13);
  expect(previewChipRightInset).toBeLessThanOrEqual(15);
  const previewChipTopInset = previewChipBox.y - previewSurfaceBox.y;
  expect(previewChipTopInset).toBeGreaterThanOrEqual(13);
  expect(previewChipTopInset).toBeLessThanOrEqual(15);
  await expect(previewContent).toHaveCSS("padding-top", "16px");
  await expect(previewContent).toHaveCSS("padding-right", "16px");
  await expect(previewContent).toHaveCSS("padding-bottom", "16px");
  await expect(previewContent).toHaveCSS("padding-left", "16px");
  expect(firstPreviewMessageBox.x - previewSurfaceBox.x).toBeGreaterThanOrEqual(
    15,
  );
  expect(firstPreviewMessageBox.x - previewSurfaceBox.x).toBeLessThanOrEqual(
    17,
  );
  expect(firstPreviewMessageBox.y - previewSurfaceBox.y).toBeGreaterThanOrEqual(
    15,
  );
  expect(firstPreviewMessageBox.y - previewSurfaceBox.y).toBeLessThanOrEqual(
    17,
  );
  await densityIndicator.evaluate((element) => {
    element.addEventListener(
      "transitionrun",
      () => element.setAttribute("data-transition-ran", "true"),
      { once: true },
    );
  });

  await compact.click();
  await expect(densityIndicator).toHaveAttribute("data-transition-ran", "true");
  await expect(root).toHaveAttribute("data-conversation-density", "compact");
  await expect(root).toHaveAttribute("data-font-size", "default");
  await expect(compact).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        CONVERSATION_DENSITY_STORAGE_KEY,
      ),
    )
    .toBe("compact");
  await expect.poll(readScale).toEqual({
    authorLineHeight: 16,
    bodyGap: 0,
    fontSize: "calc(16px * .875)",
    lineHeight: "calc(16px * 1.25)",
    paragraphGap: 0.375,
    rowPadding: 0.25,
    timestampFontSize: "calc(16px * .75)",
    timestampLineHeight: 16,
  });
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "14px",
    lineHeight: "20px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await expect.poll(readPreviewTimestampScale).toEqual(["12px", "16px"]);
  await expect.poll(readSettingsChromeScale).toEqual(["24px", "14px", "14px"]);

  await larger.click();
  await expect(root).toHaveAttribute("data-conversation-density", "compact");
  await expect(root).toHaveAttribute("data-font-size", "larger");
  await expect(larger).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FONT_SIZE_STORAGE_KEY,
      ),
    )
    .toBe("larger");
  await expect.poll(readScale).toEqual({
    authorLineHeight: 17.142857,
    bodyGap: 0,
    fontSize: "calc(17.142857px * .875)",
    lineHeight: "calc(17.142857px * 1.25)",
    paragraphGap: 0.375,
    rowPadding: 0.25,
    timestampFontSize: "calc(17.142857px * .75)",
    timestampLineHeight: 17.142857,
  });
  await expect
    .poll(() =>
      previewMessage.evaluate((element) => {
        const style = window.getComputedStyle(element);
        return [style.fontSize, style.lineHeight];
      }),
    )
    .toEqual(["15px", "21.4286px"]);
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "15px",
    lineHeight: "21.4286px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await expect
    .poll(readPreviewTimestampScale)
    .toEqual(["12.8571px", "17.1429px"]);
  await expect
    .poll(readSettingsChromeScale)
    .toEqual(["25.7143px", "15px", "15px"]);
  await waitForAnimations(page);
  await page.getByTestId("appearance-preferences-card").screenshot({
    path: `${SHOTS}/15-conversation-compact-larger.png`,
  });

  await spacious.click();
  await expect(root).toHaveAttribute("data-conversation-density", "spacious");
  await expect(root).toHaveAttribute("data-font-size", "larger");
  await expect(spacious).toHaveAttribute("aria-pressed", "true");
  await expect.poll(readScale).toEqual({
    authorLineHeight: 17.142857,
    bodyGap: 0.25,
    fontSize: "calc(17.142857px * .875)",
    lineHeight: "calc(17.142857px * 1.25)",
    paragraphGap: 0.625,
    rowPadding: 0.5,
    timestampFontSize: "calc(17.142857px * .75)",
    timestampLineHeight: 17.142857,
  });
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "15px",
    lineHeight: "21.4286px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await expect
    .poll(readPreviewTimestampScale)
    .toEqual(["12.8571px", "17.1429px"]);
  await expect
    .poll(readSettingsChromeScale)
    .toEqual(["25.7143px", "15px", "15px"]);

  await smaller.click();
  await expect(root).toHaveAttribute("data-conversation-density", "spacious");
  await expect(root).toHaveAttribute("data-font-size", "smaller");
  await expect(smaller).toHaveAttribute("aria-pressed", "true");
  await expect.poll(readScale).toEqual({
    authorLineHeight: 14.857143,
    bodyGap: 0.25,
    fontSize: "calc(14.857143px * .875)",
    lineHeight: "calc(14.857143px * 1.25)",
    paragraphGap: 0.625,
    rowPadding: 0.5,
    timestampFontSize: "calc(14.857143px * .75)",
    timestampLineHeight: 14.857143,
  });
  await expect
    .poll(() =>
      previewMessage.evaluate((element) => {
        const style = window.getComputedStyle(element);
        return [style.fontSize, style.lineHeight];
      }),
    )
    .toEqual(["13px", "18.5714px"]);
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "13px",
    lineHeight: "18.5714px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await expect
    .poll(readPreviewTimestampScale)
    .toEqual(["11.1429px", "14.8571px"]);
  await expect
    .poll(readSettingsChromeScale)
    .toEqual(["22.2857px", "13px", "13px"]);
  await waitForAnimations(page);
  await page.getByTestId("appearance-preferences-card").screenshot({
    path: `${SHOTS}/16-conversation-spacious-smaller.png`,
  });

  await comfortable.click();
  await defaultSize.click();
  await expect(root).toHaveAttribute(
    "data-conversation-density",
    "comfortable",
  );
  await expect(comfortable).toHaveAttribute("aria-pressed", "true");

  const controlBox = await densityControl.boundingBox();
  const compactBox = await compact.boundingBox();
  const spaciousBox = await spacious.boundingBox();
  expect(controlBox).not.toBeNull();
  expect(compactBox).not.toBeNull();
  expect(spaciousBox).not.toBeNull();
  if (!controlBox || !compactBox || !spaciousBox) {
    throw new Error("Conversation density control geometry is missing");
  }
  await page.mouse.move(
    compactBox.x + compactBox.width / 2,
    controlBox.y + controlBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    spaciousBox.x + spaciousBox.width / 2,
    controlBox.y + controlBox.height / 2,
  );
  await expect(root).toHaveAttribute("data-conversation-density", "spacious");
  await expect(root).toHaveAttribute("data-font-size", "default");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        CONVERSATION_DENSITY_STORAGE_KEY,
      ),
    )
    .toBe("comfortable");
  await expect.poll(readScale).toEqual({
    authorLineHeight: 16,
    bodyGap: 0.25,
    fontSize: "calc(16px * .875)",
    lineHeight: "calc(16px * 1.25)",
    paragraphGap: 0.625,
    rowPadding: 0.5,
    timestampFontSize: "calc(16px * .75)",
    timestampLineHeight: 16,
  });
  await expect.poll(readSettingsScale).toEqual({
    fontSize: "14px",
    lineHeight: "20px",
    minHeight: "64px",
    paddingBlock: "12px",
  });
  await page.mouse.up();
  await expect(spacious).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        CONVERSATION_DENSITY_STORAGE_KEY,
      ),
    )
    .toBe("spacious");
  await comfortable.click();

  const fontSizeControlBox = await fontSizeControl.boundingBox();
  const smallerBox = await smaller.boundingBox();
  const largerBox = await larger.boundingBox();
  expect(fontSizeControlBox).not.toBeNull();
  expect(smallerBox).not.toBeNull();
  expect(largerBox).not.toBeNull();
  if (!fontSizeControlBox || !smallerBox || !largerBox) {
    throw new Error("Font size control geometry is missing");
  }
  await page.mouse.move(
    smallerBox.x + smallerBox.width / 2,
    fontSizeControlBox.y + fontSizeControlBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    largerBox.x + largerBox.width / 2,
    fontSizeControlBox.y + fontSizeControlBox.height / 2,
  );
  await expect(root).toHaveAttribute("data-font-size", "larger");
  await expect(root).toHaveAttribute(
    "data-conversation-density",
    "comfortable",
  );
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FONT_SIZE_STORAGE_KEY,
      ),
    )
    .toBe("default");
  await expect
    .poll(() =>
      previewMessage.evaluate((element) => {
        const style = window.getComputedStyle(element);
        return [style.fontSize, style.lineHeight];
      }),
    )
    .toEqual(["15px", "21.4286px"]);
  await expect
    .poll(readSettingsChromeScale)
    .toEqual(["25.7143px", "15px", "15px"]);

  // Losing the window during a scrub cancels the temporary preview rather
  // than leaving presentation and persisted selection out of sync.
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(root).toHaveAttribute("data-font-size", "default");
  await expect(defaultSize).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FONT_SIZE_STORAGE_KEY,
      ),
    )
    .toBe("default");

  // Cancellation resets the gesture completely; the next selection persists.
  await larger.click();
  await expect(larger).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FONT_SIZE_STORAGE_KEY,
      ),
    )
    .toBe("larger");
  await defaultSize.click();
  await waitForAnimations(page);
  await page.getByTestId("appearance-preferences-card").screenshot({
    path: `${SHOTS}/14-conversation-preferences.png`,
  });
});

test("appearance picker — system tab (Buzz follows OS)", async ({ page }) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  const panel = await openAppearance(page, "system");
  await panel.screenshot({ path: `${SHOTS}/03-picker-system.png` });
});

test("appearance picker — light tab (Buzz)", async ({ page }) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  const panel = await openAppearance(page, "light");
  await panel.screenshot({ path: `${SHOTS}/04-picker-light.png` });
});

test("appearance picker — dark tab (Buzz Dark)", async ({ page }) => {
  await seedTheme(page, "buzz-dark");
  await installMockBridge(page);
  const panel = await openAppearance(page, "dark");
  await panel.screenshot({ path: `${SHOTS}/05-picker-dark.png` });
});

test("settings nav uses Buzz active pill + hover (light)", async ({ page }) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  const sidebar = page.getByTestId("settings-sidebar");
  await expect(sidebar).toBeVisible({ timeout: 10_000 });
  const profileRow = page.getByTestId("settings-nav-profile");
  const profileLabel = profileRow.locator('[data-sidebar="menu-label"]');
  await expect(profileRow).toHaveAttribute("data-active", "true");
  await expect(profileRow).toHaveCSS("font-weight", "600");
  const selectedLabelBox = await profileLabel.boundingBox();
  // Appearance is the active section here; its nav row should carry the Buzz
  // white active pill (data-active=true), matching the Left Nav treatment.
  await page.getByTestId("settings-nav-appearance").click();
  await expect(profileRow).toHaveCSS("font-weight", "400");
  const unselectedLabelBox = await profileLabel.boundingBox();
  expect(selectedLabelBox).not.toBeNull();
  expect(unselectedLabelBox).not.toBeNull();
  if (!selectedLabelBox || !unselectedLabelBox) {
    throw new Error("Settings nav label geometry is missing");
  }
  expect(Math.abs(selectedLabelBox.width - unselectedLabelBox.width)).toBe(0);
  await expectBuzzSettingsPalette(page, "light");
  const activeRow = page.getByTestId("settings-nav-appearance");
  await expect(activeRow).toHaveAttribute("data-active", "true");
  await waitForAnimations(page);
  await sidebar.screenshot({ path: `${SHOTS}/06-settings-nav-light.png` });
});

test("settings nav uses Buzz active pill + hover (dark)", async ({ page }) => {
  await seedTheme(page, "buzz-dark");
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  const sidebar = page.getByTestId("settings-sidebar");
  await expect(sidebar).toBeVisible({ timeout: 10_000 });
  await page.getByTestId("settings-nav-appearance").click();
  await expectBuzzSettingsPalette(page, "dark");
  await expect(page.getByTestId("settings-content-surface")).toHaveCSS(
    "background-color",
    "rgb(26, 26, 26)",
  );
  await waitForAnimations(page);
  await sidebar.screenshot({ path: `${SHOTS}/07-settings-nav-dark.png` });
  await page.getByTestId("settings-view").screenshot({
    path: `${SHOTS}/09-settings-content-dark.png`,
  });
});

test("prominent active tab is opt-in and switches selection surfaces", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await openAppearance(page, "light");

  const root = page.locator("html");
  const activeRow = page.getByTestId("settings-nav-appearance");
  const toggle = page.getByTestId("prominent-active-tab-toggle");
  await expect(toggle).not.toBeChecked();
  await expect(root).not.toHaveAttribute("data-prominent-active-tab", "");
  await expect(activeRow).toHaveCSS("background-color", "rgba(0, 0, 0, 0.07)");
  const subtleTextStyle = await activeRow.evaluate((element) => {
    const styles = getComputedStyle(element);
    return { color: styles.color, fontWeight: styles.fontWeight };
  });
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        PROMINENT_ACTIVE_TAB_STORAGE_KEY,
      ),
    )
    .toBeNull();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(root).toHaveAttribute("data-prominent-active-tab", "");
  await expect(activeRow).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.82)",
  );
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        PROMINENT_ACTIVE_TAB_STORAGE_KEY,
      ),
    )
    .toBe("true");
  const prominentTextStyle = await activeRow.evaluate((element) => {
    const styles = getComputedStyle(element);
    return { color: styles.color, fontWeight: styles.fontWeight };
  });
  expect(prominentTextStyle).toEqual(subtleTextStyle);

  await toggle.click();
  await expect(root).not.toHaveAttribute("data-prominent-active-tab", "");
  await expect(activeRow).toHaveCSS("background-color", "rgba(0, 0, 0, 0.07)");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        PROMINENT_ACTIVE_TAB_STORAGE_KEY,
      ),
    )
    .toBe("false");
});

test("prominent channel and direct-message rows share one flat active state", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await page.addInitScript(
    ({ key }) => window.localStorage.setItem(key, "true"),
    { key: PROMINENT_ACTIVE_TAB_STORAGE_KEY },
  );
  await installMockBridge(page);
  await openChannel(page);

  const channelRow = page.getByTestId("channel-general");
  const directMessageRow = page.getByTestId("channel-alice-tyler");

  await expect(channelRow).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.82)",
  );
  await expect(channelRow).toHaveCSS("box-shadow", "none");
  await channelRow.hover();
  await expect(channelRow).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.82)",
  );

  await directMessageRow.click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect(directMessageRow).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.82)",
  );
  await expect(directMessageRow).toHaveCSS("box-shadow", "none");
  await directMessageRow.hover();
  await expect(directMessageRow).toHaveCSS(
    "background-color",
    "rgba(255, 255, 255, 0.82)",
  );
});

for (const { activeSurface, hoverSurface, mode, theme } of [
  {
    activeSurface: "rgba(0, 0, 0, 0.07)",
    hoverSurface: "rgba(0, 0, 0, 0.04)",
    mode: "light" as const,
    theme: "buzz",
  },
  {
    activeSurface: "rgba(255, 255, 255, 0.16)",
    hoverSurface: "rgba(255, 255, 255, 0.04)",
    mode: "dark" as const,
    theme: "buzz-dark",
  },
]) {
  test(`non-prominent ${theme} selection matches production`, async ({
    page,
  }) => {
    await seedTheme(page, theme);
    await page.addInitScript(
      ({ key }) => window.localStorage.setItem(key, "false"),
      { key: PROMINENT_ACTIVE_TAB_STORAGE_KEY },
    );
    await installMockBridge(page);
    await openChannel(page);

    const root = page.locator("html");
    const sidebar = page.getByTestId("app-sidebar");
    const activeRow = page.getByTestId("channel-general");
    await expect(root).toHaveClass(
      new RegExp(`(^|\\s)${mode === "dark" ? "dark" : "light"}($|\\s)`),
    );
    await expect(root).not.toHaveAttribute("data-prominent-active-tab", "");
    await expect(activeRow).toHaveCSS("background-color", activeSurface);
    await expect(activeRow).toHaveCSS("box-shadow", "none");
    await expect(activeRow).toHaveCSS("font-weight", "400");
    await activeRow.hover();
    await expect(activeRow).toHaveCSS("background-color", activeSurface);
    const inactiveRow = page.getByTestId("channel-random");
    await inactiveRow.hover();
    await expect(inactiveRow).toHaveCSS("background-color", hoverSurface);
    const expectedForeground = await sidebar.evaluate((element) => {
      const probe = document.createElement("span");
      probe.style.color = "hsl(var(--sidebar-active-foreground))";
      element.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    });
    await expect(activeRow).toHaveCSS("color", expectedForeground);
  });
}

for (const { mode, theme } of [
  { mode: "light" as const, theme: "github-light" },
  { mode: "dark" as const, theme: "github-dark" },
]) {
  test(`${theme} ignores the Buzz prominent preference`, async ({ page }) => {
    await seedTheme(page, theme);
    await page.addInitScript(
      ({ key }) => window.localStorage.setItem(key, "true"),
      { key: PROMINENT_ACTIVE_TAB_STORAGE_KEY },
    );
    await installMockBridge(page);
    await openAppearance(page, mode);

    const root = page.locator("html");
    const activeRow = page.getByTestId("settings-nav-appearance");
    await expect(page.getByTestId("prominent-active-tab-row")).toHaveCount(0);
    await expect(root).not.toHaveAttribute("data-prominent-active-tab", "");

    const productionStyle = await page.evaluate(() => {
      const sidebar = document.querySelector<HTMLElement>(
        '[data-testid="settings-sidebar"]',
      );
      const row = document.querySelector<HTMLElement>(
        '[data-testid="settings-nav-appearance"]',
      );
      if (!sidebar || !row) return null;
      const probe = document.createElement("span");
      probe.style.backgroundColor = "hsl(var(--sidebar-active))";
      probe.style.color = "hsl(var(--sidebar-active-foreground))";
      sidebar.append(probe);
      const probeStyles = getComputedStyle(probe);
      const rowStyles = getComputedStyle(row);
      const result = {
        expectedBackground: probeStyles.backgroundColor,
        expectedForeground: probeStyles.color,
        background: rowStyles.backgroundColor,
        color: rowStyles.color,
        fontWeight: rowStyles.fontWeight,
      };
      probe.remove();
      return result;
    });

    expect(productionStyle).not.toBeNull();
    expect(productionStyle?.background).toBe(
      productionStyle?.expectedBackground,
    );
    expect(productionStyle?.color).toBe(productionStyle?.expectedForeground);
    await expect(activeRow).toHaveCSS(
      "font-weight",
      productionStyle?.fontWeight ?? "",
    );
  });
}

test("settings content uses the same inset surface as the main app", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  const searchBox = await page.getByTestId("open-search").boundingBox();
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();

  const settingsView = page.getByTestId("settings-view");
  const contentSurface = page.getByTestId("settings-content-surface");
  const settingsTopChrome = page.getByTestId("settings-top-chrome");
  const settingsSidebarTopChrome = page.getByTestId(
    "settings-sidebar-top-chrome",
  );
  const backToAppBox = await page
    .getByTestId("settings-back-to-app")
    .boundingBox();
  await expect(contentSurface).toBeVisible({ timeout: 10_000 });
  for (const dragRegion of [settingsTopChrome, settingsSidebarTopChrome]) {
    await expect(dragRegion).toHaveAttribute(
      "data-tauri-drag-region",
      /^(?:|true)$/,
    );
    await expect(dragRegion).toHaveCSS("cursor", "default");
    await expect(dragRegion).toHaveCSS("user-select", "none");
  }
  await expect(page.getByTestId("settings-content-scroll")).toHaveCSS(
    "padding-top",
    "24px",
  );

  const viewBox = await settingsView.boundingBox();
  const surfaceBox = await contentSurface.boundingBox();
  expect(searchBox).not.toBeNull();
  expect(backToAppBox).not.toBeNull();
  expect(viewBox).not.toBeNull();
  expect(surfaceBox).not.toBeNull();
  if (!searchBox || !backToAppBox || !viewBox || !surfaceBox) {
    throw new Error("Settings layout is missing");
  }

  expect(Math.abs(backToAppBox.y - searchBox.y)).toBeLessThanOrEqual(0.5);

  // Match the normal app shell: a fixed 40px top chrome strip, then a 1px
  // top/left inset and 8px right/bottom inset around the rounded content card.
  expect(surfaceBox.y - viewBox.y).toBe(41);
  expect(surfaceBox.x - viewBox.x).toBe(1);
  expect(viewBox.x + viewBox.width - (surfaceBox.x + surfaceBox.width)).toBe(8);
  expect(viewBox.y + viewBox.height - (surfaceBox.y + surfaceBox.height)).toBe(
    8,
  );

  const topChromeBox = await settingsTopChrome.boundingBox();
  const settingsHeadingBox = await page
    .getByRole("heading", { level: 1, name: "Profile" })
    .boundingBox();
  expect(topChromeBox).not.toBeNull();
  expect(settingsHeadingBox).not.toBeNull();
  if (!topChromeBox || !settingsHeadingBox) {
    throw new Error("Settings drag region or title is missing");
  }
  await page.mouse.move(
    topChromeBox.x + topChromeBox.width / 2,
    topChromeBox.y + topChromeBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    settingsHeadingBox.x + settingsHeadingBox.width / 2,
    settingsHeadingBox.y + settingsHeadingBox.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("");

  await waitForAnimations(page);
  await settingsView.screenshot({
    path: `${SHOTS}/08-settings-content-inset.png`,
  });
});

test("appearance hides accent picker under Buzz", async ({ page }) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  const panel = await openAppearance(page, "light");
  // The accent picker is hidden while a Buzz theme is active. Its neutral
  // swatch testid must not be present.
  await expect(page.getByTestId("accent-color-neutral")).toHaveCount(0);
  await panel.screenshot({ path: `${SHOTS}/10-appearance-no-accent.png` });
});

test("glass background keeps the content panel solid", async ({ page }) => {
  await seedTheme(page, "buzz");
  await page.addInitScript(() => {
    (window as typeof window & { isTauri?: boolean }).isTauri = true;
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "MacIntel",
    });
  });
  await installMockBridge(page);
  const panel = await openAppearance(page, "light");

  const toggle = page.getByTestId("glass-background-toggle");
  const opacitySlider = page.getByTestId("glass-opacity-slider");
  const root = page.locator("html");
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        GLASS_BACKGROUND_STORAGE_KEY,
      ),
    )
    .toBeNull();
  await expect(opacitySlider).toHaveCount(0);
  await expect(root).not.toHaveAttribute("data-glass-background", "");
  await expect(page.locator(".buzz-theme-gradient-underlay")).not.toHaveCSS(
    "background-image",
    "none",
  );

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(opacitySlider).toBeVisible();
  await expect(opacitySlider).toHaveClass(/buzz-avatar-framing-slider/);
  await expect(opacitySlider).toHaveAttribute("aria-valuenow", "65");
  await expect(opacitySlider).toHaveCSS("height", "32px");
  const matchingRadiusControls = [
    page.getByTestId("appearance-color-mode-control"),
    page.getByTestId("appearance-color-mode-indicator"),
    page.getByTestId("font-size-control"),
    page.getByTestId("font-size-control-indicator"),
    page.getByTestId("conversation-density-control"),
    page.getByTestId("conversation-density-control-indicator"),
    page.getByTestId("theme-style-trigger"),
    page.getByTestId("link-preview-style-control"),
    page.getByTestId("link-preview-style-control-indicator"),
    page.getByTestId("thread-layout-control"),
    page.getByTestId("thread-layout-control-indicator"),
  ];
  for (const control of matchingRadiusControls) {
    await expect(control).toHaveCSS("border-radius", "8px");
  }
  await expect(page.getByTestId("glass-opacity-value")).toHaveCount(0);
  await expect(
    opacitySlider.locator(".buzz-avatar-framing-slider-handle"),
  ).toHaveCSS("opacity", "1");
  await expect(root).toHaveAttribute("data-glass-background", "");
  const buzzSettingOrder = await page
    .getByTestId("appearance-theme-card")
    .locator(
      '[data-testid="appearance-color-mode-row"], [data-testid="theme-style-row"], [data-testid="glass-background-row"], [data-testid="glass-opacity-row"], [data-testid="prominent-active-tab-row"]',
    )
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-testid")));
  expect(buzzSettingOrder).toEqual([
    "appearance-color-mode-row",
    "theme-style-row",
    "glass-background-row",
    "glass-opacity-row",
    "prominent-active-tab-row",
  ]);
  await expect(page.locator(".buzz-theme-gradient-underlay")).toHaveCSS(
    "background-image",
    "none",
  );
  for (const canvas of [root, page.locator("body"), page.locator("#root")]) {
    await expect(canvas).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  }
  await expect(
    page.getByTestId("settings-sidebar").locator('[data-sidebar="sidebar"]'),
  ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(page.getByTestId("settings-content-surface")).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
          (entry) =>
            entry.command === "set_window_vibrancy" &&
            (entry.payload as { enabled?: boolean } | undefined)?.enabled ===
              true,
        ),
      ),
    )
    .toBe(true);

  await opacitySlider.press("Home");
  await expect(opacitySlider).toHaveAttribute("aria-valuenow", "30");
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.documentElement.style.getPropertyValue(
          "--glass-background-opacity",
        ),
      ),
    )
    .toBe("30%");
  await expect
    .poll(() =>
      page.evaluate(
        (storageKey) => localStorage.getItem(storageKey),
        GLASS_OPACITY_STORAGE_KEY,
      ),
    )
    .toBe("30");
  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/11-appearance-glass.png` });

  await toggle.click();
  await expect(root).not.toHaveAttribute("data-glass-background", "");
  await expect(opacitySlider).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        (storageKey) => localStorage.getItem(storageKey),
        GLASS_BACKGROUND_STORAGE_KEY,
      ),
    )
    .toBe("false");
});

test("glass background is unavailable on Linux", async ({ page }) => {
  await seedTheme(page, "buzz");
  await page.addInitScript((storageKey) => {
    window.localStorage.setItem(storageKey, "true");
    (window as typeof window & { isTauri?: boolean }).isTauri = true;
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "Linux x86_64",
    });
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      get: () => "Buzz Desktop Linux",
    });
  }, GLASS_BACKGROUND_STORAGE_KEY);
  await installMockBridge(page);
  await openAppearance(page, "light");

  await expect(page.getByTestId("glass-background-row")).toHaveCount(0);
  await expect(page.getByTestId("glass-background-toggle")).toHaveCount(0);
  await expect(page.getByTestId("glass-opacity-slider")).toHaveCount(0);
  await expect(page.locator("html")).not.toHaveAttribute(
    "data-glass-background",
    "",
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
          (entry) => entry.command === "set_window_vibrancy",
        ),
      ),
    )
    .toBe(false);
  await expect
    .poll(() =>
      page.evaluate(
        (storageKey) => window.localStorage.getItem(storageKey),
        GLASS_BACKGROUND_STORAGE_KEY,
      ),
    )
    .toBe("true");
});

test("non-Buzz glass preserves the selected theme sidebar tint", async ({
  page,
}) => {
  await seedTheme(page, "rose-pine-dawn");
  await page.addInitScript(() => {
    (window as typeof window & { isTauri?: boolean }).isTauri = true;
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "MacIntel",
    });
  });
  await installMockBridge(page);
  await openAppearance(page, "light");

  const root = page.locator("html");
  await expect(root).not.toHaveAttribute("data-buzz-sidebar", "");
  await page.getByTestId("glass-background-toggle").click();
  await expect(root).toHaveAttribute("data-glass-background", "");

  const tint = await page
    .locator(".buzz-theme-gradient-layer")
    .evaluate((element) => {
      const rootStyles = getComputedStyle(document.documentElement);
      const sidebar = rootStyles
        .getPropertyValue("--sidebar-background")
        .trim();
      const staticFallback = rootStyles.getPropertyValue("--sidebar").trim();
      const probe = document.createElement("div");
      probe.style.backgroundColor = `hsl(${sidebar} / 65%)`;
      document.body.appendChild(probe);
      const expected = getComputedStyle(probe).backgroundColor;
      probe.remove();

      return {
        actual: getComputedStyle(element).backgroundColor,
        expected,
        sidebar,
        staticFallback,
      };
    });

  expect(tint.sidebar).not.toBe(tint.staticFallback);
  expect(tint.actual).toBe(tint.expected);
});

test("accent picker reveals/hides when toggling Buzz", async ({ page }) => {
  // Start on a non-Buzz theme so the accent picker is present, then select the
  // Buzz tile — the picker should animate out and unmount. Reselecting a
  // non-Buzz tile brings it back. Asserts the presence toggle (the motion
  // wrapper) works end to end.
  await seedTheme(page, "github-light");
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => "MacIntel",
    });
  });
  await installMockBridge(page);
  await openAppearance(page, "light");
  await expect(page.getByTestId("accent-color-neutral")).toBeVisible();
  const nonBuzzSettingOrder = await page
    .getByTestId("appearance-theme-card")
    .locator(
      '[data-testid="appearance-color-mode-row"], [data-testid="theme-style-row"], [data-testid="accent-color-options"], [data-testid="glass-background-row"], [data-testid="prominent-active-tab-row"]',
    )
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-testid")));
  expect(nonBuzzSettingOrder).toEqual([
    "appearance-color-mode-row",
    "theme-style-row",
    "accent-color-options",
    "glass-background-row",
  ]);

  // Switch to Buzz — picker should leave (allow the exit animation to settle).
  await page.getByTestId("theme-style-trigger").click();
  await page.getByTestId("theme-option-buzz").click();
  await expect(page.getByTestId("theme-style-trigger")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expect(page.getByTestId("accent-color-neutral")).toHaveCount(0);

  // Back to a non-Buzz theme — picker returns.
  await page.getByTestId("theme-option-github-light").click();
  await expect(page.getByTestId("accent-color-neutral")).toBeVisible();
  await expect(page.getByTestId("theme-style-trigger")).toHaveAttribute(
    "aria-expanded",
    "true",
  );

  const swatchBoxes = await page
    .getByTestId("accent-color-options")
    .locator("button")
    .evaluateAll((buttons) =>
      buttons.map((button) => {
        const box = button.getBoundingClientRect();
        return { x: box.x, y: box.y };
      }),
    );
  expect(swatchBoxes).toHaveLength(10);
  expect(new Set(swatchBoxes.map((box) => Math.round(box.y))).size).toBe(1);
  await expect(page.getByTestId("accent-color-options")).toHaveCSS(
    "overflow-x",
    "auto",
  );
  await expect(page.getByTestId("accent-color-blue")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(
    page.getByTestId("accent-color-blue").getByTestId("accent-color-selection"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("settings-theme").screenshot({
    path: `${SHOTS}/12-appearance-theme-and-accents.png`,
  });
  const accentOptions = page.getByTestId("accent-color-options");
  await accentOptions.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await accentOptions.screenshot({
    path: `${SHOTS}/13-appearance-accent-row.png`,
  });
});

test("Buzz light and dark modes apply live without a reload", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await installMockBridge(page);
  await openAppearance(page, "light");
  await expectAppliedBuzzTheme(page, "buzz");
  const lightGradient = await expectBuzzGradientPaint(page, "light");

  await page.getByTestId("appearance-mode-dark").click();
  await expectAppliedBuzzTheme(page, "buzz-dark");
  const darkGradient = await expectBuzzGradientPaint(page, "dark");
  expect(darkGradient).not.toBe(lightGradient);

  await page.getByTestId("appearance-mode-light").click();
  await expectAppliedBuzzTheme(page, "buzz");
  await expectBuzzGradientPaint(page, "light");

  // Exercise the overlap that previously let a slower, stale theme load win.
  await page.getByTestId("appearance-mode-dark").click();
  await page.getByTestId("appearance-mode-light").click();
  await expectAppliedBuzzTheme(page, "buzz");
});

test("Buzz follows native system theme changes without a reload", async ({
  page,
}) => {
  await seedTheme(page, "buzz");
  await page.addInitScript(() => {
    (window as typeof window & { isTauri?: boolean }).isTauri = true;
  });
  await installMockBridge(page);
  await openAppearance(page, "system");

  await emitNativeThemeChange(page, "dark");
  await expectAppliedBuzzTheme(page, "buzz-dark", "buzz");
  await expectBuzzGradientPaint(page, "dark");

  await emitNativeThemeChange(page, "light");
  await expectAppliedBuzzTheme(page, "buzz", "buzz");
  await expectBuzzGradientPaint(page, "light");
});
