import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/buzz-theme";
const THEME_STORAGE_KEY = "buzz-theme";
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
  const hoverSurface =
    mode === "light" ? "rgba(0, 0, 0, 0.04)" : "rgba(255, 255, 255, 0.04)";
  const activeSurface =
    mode === "light" ? "rgba(0, 0, 0, 0.07)" : "color(srgb 1 1 1 / 0.16)";
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
    "background-color",
    activeSurface,
  );
  const hoverChannel = page.getByTestId("channel-random");
  await hoverChannel.hover();
  await expect(hoverChannel).toHaveCSS("background-color", hoverSurface);

  const firstDmItem = page
    .getByTestId("dm-list")
    .locator('[data-sidebar="menu-item"]')
    .first();
  const firstDmButton = firstDmItem.locator('[data-sidebar="menu-button"]');
  const closeDmButton = firstDmItem.getByRole("button", {
    name: "Close direct message",
  });
  await firstDmItem.hover();
  await expect(closeDmButton).toBeVisible();
  await closeDmButton.hover();
  await expect(firstDmButton).toHaveCSS("background-color", hoverSurface);

  const scrollbarThumbColor = await sidebarScroller.evaluate(
    (element) =>
      getComputedStyle(element, "::-webkit-scrollbar-thumb").backgroundColor,
  );
  expect(scrollbarThumbColor).toBe(hoverSurface);
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
  const backToAppBox = await page
    .getByTestId("settings-back-to-app")
    .boundingBox();
  await expect(contentSurface).toBeVisible({ timeout: 10_000 });
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

test("accent picker reveals/hides when toggling Buzz", async ({ page }) => {
  // Start on a non-Buzz theme so the accent picker is present, then select the
  // Buzz tile — the picker should animate out and unmount. Reselecting a
  // non-Buzz tile brings it back. Asserts the presence toggle (the motion
  // wrapper) works end to end.
  await seedTheme(page, "github-light");
  await installMockBridge(page);
  await openAppearance(page, "light");
  await expect(page.getByTestId("accent-color-neutral")).toBeVisible();

  // Switch to Buzz — picker should leave (allow the exit animation to settle).
  await page.getByTestId("theme-option-buzz").click();
  await expect(page.getByTestId("accent-color-neutral")).toHaveCount(0);

  // Back to a non-Buzz theme — picker returns.
  await page.getByTestId("theme-option-github-light").click();
  await expect(page.getByTestId("accent-color-neutral")).toBeVisible();
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
