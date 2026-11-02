import { bytesToHex } from "@noble/hashes/utils.js";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { generateSecretKey, getPublicKey } from "nostr-tools/pure";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";
import { openSettings } from "../helpers/settings";
import { makeCofounderNumbersFixture } from "../../src/testing/cofounderNumbersFixtures";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const RELAY_URL = (
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000"
).replace(/^http/u, "ws");
const COMMUNITY_ICON =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 40 40'%3E%3Crect width='40' height='40' rx='12' fill='%23f5a4d5'/%3E%3Cg fill='%2325222c'%3E%3Ccircle cx='12' cy='20' r='4'/%3E%3Ccircle cx='20' cy='20' r='5'/%3E%3Ccircle cx='29' cy='20' r='4'/%3E%3C/g%3E%3C/svg%3E";
const HOLD_COMMUNITY_ICON_FETCH_MS = 30_000;
const CAPTURE_DIR = process.env.WS_HEADER_CAPTURE_DIR;
const CAPTURE_PHASE = process.env.WS_HEADER_CAPTURE_PHASE ?? "capture";
const COMMUNITY_THEME_KEY =
  "buzz-community-theme.v1:" +
  MOCK_PUBKEY +
  ":" +
  encodeURIComponent(RELAY_URL);

async function seedWorkspace(
  page: Page,
  options: {
    icon?: string;
    logoUrl?: string;
    website?: string;
    theme?: "light" | "dark";
    mock?: NonNullable<Parameters<typeof installMockBridge>[1]>;
  } = {},
) {
  const theme = options.theme === "dark" ? "buzz-dark" : "buzz";
  await page.addInitScript(
    ({
      communityThemeKey,
      icon,
      logoUrl,
      relayUrl,
      theme,
      pubkey,
      website,
    }) => {
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify([
          {
            id: "workspace-header-icon",
            name: "Colony",
            relayUrl,
            pubkey,
            addedAt: "2026-01-01T00:00:00.000Z",
          },
        ]),
      );
      window.localStorage.setItem(
        "buzz-active-community-id",
        "workspace-header-icon",
      );
      window.localStorage.setItem(
        "buzz-community-icons",
        JSON.stringify(icon ? { [relayUrl]: icon } : {}),
      );
      if (logoUrl) {
        window.localStorage.setItem(
          `colony-business-profile.v1:${"workspace-header-icon"}`,
          JSON.stringify({
            name: "Colony",
            website,
            description: "",
            logoDataUrl: null,
            logoUrl,
          }),
        );
      }
      window.localStorage.setItem("buzz-theme", theme);
      window.localStorage.setItem("buzz-follow-system", "false");
      window.localStorage.setItem(
        communityThemeKey,
        JSON.stringify({
          version: 1,
          theme,
          accent: "#5B45BE",
          followSystem: false,
        }),
      );
    },
    {
      communityThemeKey: COMMUNITY_THEME_KEY,
      icon: options.icon ?? null,
      logoUrl: options.logoUrl ?? null,
      relayUrl: RELAY_URL,
      theme,
      pubkey: MOCK_PUBKEY,
      website:
        options.website ?? (options.logoUrl ? "https://colony.example" : ""),
    },
  );
  await installMockBridge(
    page,
    {
      ...options.mock,
      commandDelaysMs: {
        ...options.mock?.commandDelaysMs,
        fetch_workspace_icon: HOLD_COMMUNITY_ICON_FETCH_MS,
      },
    },
    { skipCommunitySeed: true },
  );
}

const MOCK_AYA_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const MOCK_SCOUT_PUBKEY = TEST_IDENTITIES.bob.pubkey;

function companyMapMock(): NonNullable<
  Parameters<typeof installMockBridge>[1]
> {
  const relaySecret = generateSecretKey();
  return {
    relaySelf: getPublicKey(relaySecret),
    relayRequiresMembership: true,
    relayRole: "owner",
    relayMembers: [
      { pubkey: MOCK_PUBKEY, role: "owner" },
      { pubkey: MOCK_AYA_PUBKEY, role: "member" },
    ],
    companyMemberRelayPrivateKeyHex: bytesToHex(relaySecret),
    cofounderSettings: {
      enabled: true,
      moneyPreference: "cheapest_first",
      approvalSwitches: {
        approveEveryHire: false,
        approvePlaybookChanges: false,
        approveEveryDiscoveryCampaign: false,
      },
    },
    companyNumbers: makeCofounderNumbersFixture({
      ownerPubkey: MOCK_PUBKEY,
      scoutPubkey: MOCK_SCOUT_PUBKEY,
      salesLeadPubkey: MOCK_AYA_PUBKEY,
    }),
    companyFunctionHeads: [
      { function: "sales", state: "filled", leadPubkey: MOCK_AYA_PUBKEY },
      { function: "operations", state: "covered_by_scout", leadPubkey: null },
    ],
    searchProfiles: [
      { pubkey: MOCK_AYA_PUBKEY, displayName: "Aya", isAgent: true },
      { pubkey: MOCK_SCOUT_PUBKEY, displayName: "Scout", isAgent: true },
    ],
    relayAgents: [
      {
        pubkey: MOCK_AYA_PUBKEY,
        ownerPubkey: MOCK_PUBKEY,
        name: "Aya",
        agentType: "agent",
        status: "online",
      },
      {
        pubkey: MOCK_SCOUT_PUBKEY,
        ownerPubkey: MOCK_PUBKEY,
        name: "Scout",
        agentType: "agent",
        status: "online",
      },
    ],
    managedAgents: [
      {
        pubkey: MOCK_AYA_PUBKEY,
        name: "Aya",
        status: "running",
        channelNames: ["general"],
      },
      {
        pubkey: MOCK_SCOUT_PUBKEY,
        name: "Scout",
        status: "running",
        channelNames: ["general"],
      },
    ],
  };
}

const OWNED_BUSINESS = {
  id: "owned-business",
  name: "Owned Business",
  slug: "owned-business",
  normalized_host: "owned-business.example.test",
  owner_pubkey: TEST_IDENTITIES.tyler.pubkey,
};
const OWNED_BUSINESS_RELAY_URL = `wss://${OWNED_BUSINESS.normalized_host}`;

async function openWelcomeOwnedBusinessList(
  page: Page,
  options: {
    theme?: "light" | "dark";
    viewport?: { width: number; height: number };
  } = {},
) {
  const theme = options.theme === "dark" ? "buzz-dark" : "buzz";
  await page.setViewportSize(options.viewport ?? { width: 1440, height: 960 });
  await page.addInitScript(
    ({ icon, theme, pubkey, ownedBusinessRelayUrl }) => {
      localStorage.setItem("buzz-theme", theme);
      localStorage.setItem("buzz-follow-system", "false");
      localStorage.setItem("buzz-active-community-id", "");
      localStorage.setItem("buzz-communities", "[]");
      localStorage.setItem(
        "buzz-community-icons",
        JSON.stringify(icon ? { [ownedBusinessRelayUrl]: icon } : {}),
      );
      localStorage.setItem(
        `buzz-community-theme.v1:${pubkey}:${encodeURIComponent("ws://localhost:3000")}`,
        JSON.stringify({
          version: 1,
          theme,
          accent: "#5B45BE",
          followSystem: false,
        }),
      );
      localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
    },
    {
      icon: COMMUNITY_ICON,
      theme,
      pubkey: TEST_IDENTITIES.tyler.pubkey,
      ownedBusinessRelayUrl: OWNED_BUSINESS_RELAY_URL,
    },
  );
  await seedActiveIdentity(page, { ...TEST_IDENTITIES.tyler, username: "" });
  await installMockBridge(
    page,
    {
      accountLinked: true,
      commandDelaysMs: {
        fetch_workspace_icon: HOLD_COMMUNITY_ICON_FETCH_MS,
      },
    },
    {
      relayWsUrl: "ws://localhost:3000",
      skipOnboardingSeed: true,
      skipCommunitySeed: true,
    },
  );
  await page.route("**/api/communities/config", (route) =>
    route.fulfill({
      json: {
        self_serve: true,
        domain: "example.test",
        public: true,
        max_per_owner: 3,
      },
    }),
  );
  await page.route("**/api/communities/mine", (route) =>
    route.fulfill({
      json: {
        owner_pubkey: TEST_IDENTITIES.tyler.pubkey,
        communities: [OWNED_BUSINESS],
      },
    }),
  );
  await page.goto("/");
  await expect(page.getByTestId("welcome-setup")).toBeVisible();
  await expect(
    page.getByTestId(`welcome-owned-business-${OWNED_BUSINESS.id}`),
  ).toBeVisible();
}

const SIGN_IN_BUSINESS_ID = "signed-in-business";
const SIGN_IN_RELAY_URL = "wss://signed-in-business.accounts-test.example";
const BUSINESS_LOGO =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 40 40'%3E%3Crect width='40' height='40' rx='12' fill='%2368c8bf'/%3E%3C/svg%3E";

async function openReturningBusinessList(
  page: Page,
  options: {
    theme?: "light" | "dark";
    capture?: string;
    viewport?: { width: number; height: number };
    includeProfileLogo?: boolean;
    communityIcon?: string | null;
    holdCommunityIconFetch?: boolean;
  } = {},
) {
  const theme = options.theme === "dark" ? "buzz-dark" : "buzz";
  await page.addInitScript(
    ({ icon, profileKey, relayUrl, theme, profileLogo }) => {
      window.localStorage.setItem(
        "buzz-community-icons",
        JSON.stringify(icon ? { [relayUrl]: icon } : {}),
      );
      window.localStorage.setItem("buzz-theme", theme);
      window.localStorage.setItem("buzz-follow-system", "false");
      window.localStorage.setItem(
        profileKey,
        JSON.stringify({
          name: "Lerato Social",
          website: "https://lerato.example",
          description: "",
          logoDataUrl: null,
          logoUrl: profileLogo,
        }),
      );
    },
    {
      icon:
        options.communityIcon === undefined
          ? COMMUNITY_ICON
          : options.communityIcon,
      profileKey: `colony-business-profile.v1:${SIGN_IN_BUSINESS_ID}`,
      relayUrl: SIGN_IN_RELAY_URL,
      theme,
      profileLogo: options.includeProfileLogo ? BUSINESS_LOGO : null,
    },
  );
  await installMockBridge(
    page,
    { accountLinked: true, accountEmail: "lerato@example.com" },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.route("**/api/communities/config", (route) =>
    route.fulfill({
      json: {
        self_serve: true,
        domain: "accounts-test.example",
        public: true,
        max_per_owner: 3,
      },
    }),
  );
  await page.route("**/api/communities/mine", (route) =>
    route.fulfill({
      json: {
        owner_pubkey: MOCK_PUBKEY,
        communities: [
          {
            id: SIGN_IN_BUSINESS_ID,
            name: "Lerato Social",
            slug: "lerato-social",
            normalized_host: new URL(SIGN_IN_RELAY_URL).hostname,
            owner_pubkey: MOCK_PUBKEY,
          },
        ],
      },
    }),
  );
  await page.setViewportSize(options.viewport ?? { width: 1440, height: 960 });
  await page.goto("/");
  await expect(page.getByTestId("onboarding-scene-account")).toBeVisible();
  if (options.holdCommunityIconFetch) {
    await page.evaluate((icon) => {
      type IconFetchWindow = Window & {
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            args?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
        __workspaceIconFetchPending?: boolean;
        __releaseWorkspaceIconFetch?: () => void;
      };
      const target = window as IconFetchWindow;
      const internals = target.__TAURI_INTERNALS__;
      if (!internals?.invoke) {
        throw new Error("Tauri mock invoke is unavailable");
      }
      const invoke = internals.invoke.bind(internals);
      target.__workspaceIconFetchPending = false;
      internals.invoke = (command, args) => {
        if (command !== "fetch_workspace_icon") {
          return invoke(command, args);
        }
        target.__workspaceIconFetchPending = true;
        return new Promise((resolve) => {
          target.__releaseWorkspaceIconFetch = () => {
            target.__workspaceIconFetchPending = false;
            target.__releaseWorkspaceIconFetch = undefined;
            resolve(icon);
          };
        });
      };
    }, COMMUNITY_ICON);
  }
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByTestId("onboarding-scene-signin")).toBeVisible();
  await page.getByLabel("Email address").fill("lerato@example.com");
  await page.locator("#signin-password").fill("studio-afternoons");
  await page.getByTestId("account-auth-submit-signin").click();
  await expect(page.getByTestId("onboarding-business-list")).toBeVisible();
  if (options.holdCommunityIconFetch) {
    await page.waitForFunction(() => {
      type IconFetchWindow = Window & {
        __workspaceIconFetchPending?: boolean;
      };
      return (window as IconFetchWindow).__workspaceIconFetchPending === true;
    });
  }

  if (options.capture && CAPTURE_DIR) {
    mkdirSync(CAPTURE_DIR, { recursive: true });
    await waitForAnimations(page);
    await page.screenshot({
      fullPage: false,
      path: join(CAPTURE_DIR, `${CAPTURE_PHASE}-${options.capture}.png`),
    });
  }
}

async function openSidebar(page: Page) {
  const businessSwitcher = page.getByTestId("sidebar-business-switcher");
  const viewport = page.viewportSize();
  if (viewport && viewport.width >= 768) {
    await setDesktopSidebarCollapsed(page, false);
  }
  if (!(await businessSwitcher.isVisible())) {
    const mobileSidebarTrigger = page
      .locator('[data-sidebar="trigger"]:visible')
      .first();
    await expect(mobileSidebarTrigger).toBeVisible();
    await mobileSidebarTrigger.click();
  }
  await expect(businessSwitcher).toBeVisible();
}

async function setDesktopSidebarCollapsed(page: Page, collapsed: boolean) {
  const viewport = page.viewportSize();
  if (!viewport || viewport.width < 768) return;
  const sidebar = page.getByTestId("app-sidebar");
  const sidebarShell = sidebar.locator("xpath=..");
  const currentCollapsed =
    (await sidebarShell.getAttribute("data-collapsible")) === "icon";
  if (currentCollapsed !== collapsed) {
    await sidebar.locator(".colony-sidebar-collapse").click();
  }
  if (collapsed) {
    await expect(sidebarShell).toHaveAttribute("data-collapsible", "icon");
  } else {
    await expect(sidebarShell).not.toHaveAttribute("data-collapsible", "icon");
  }
}

async function openWorkspaceMenuItem(page: Page) {
  await page.getByTestId("sidebar-business-switcher").click();
  const workspaceMenu = page.getByTestId("workspace-switcher-menu");
  await expect(workspaceMenu).toBeVisible();
  return workspaceMenu.getByTestId("workspace-menu-item-workspace-header-icon");
}

test("header icon matches the workspace menu and remains in the collapsed sidebar", async ({
  page,
}) => {
  await seedWorkspace(page, { icon: COMMUNITY_ICON });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/today");
  await openSidebar(page);

  const brandMark = page
    .getByTestId("sidebar-business-switcher")
    .getByTestId("sidebar-community-brand-mark");
  const headerImage = brandMark.locator("img");
  await expect(headerImage).toHaveCount(1);
  await expect(headerImage).toBeVisible();
  await expect(headerImage).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(brandMark.locator(".colony-sidebar-brand-dot")).toHaveCount(0);

  const workspaceMenuItem = await openWorkspaceMenuItem(page);
  const switcherImage = workspaceMenuItem.locator("img");
  await expect(switcherImage).toBeVisible();
  expect(await headerImage.getAttribute("src")).toBe(
    await switcherImage.getAttribute("src"),
  );

  await page.keyboard.press("Escape");
  await setDesktopSidebarCollapsed(page, true);
  await expect(headerImage).toBeVisible();
  const tile = await brandMark.boundingBox();
  expect(tile?.width).toBe(40);
  expect(tile?.height).toBe(40);
});

test("header keeps the letter tile when the community has no icon", async ({
  page,
}) => {
  await seedWorkspace(page);
  await page.goto("/#/today");
  await openSidebar(page);

  const brandMark = page
    .getByTestId("sidebar-business-switcher")
    .getByTestId("sidebar-community-brand-mark");
  await expect(
    brandMark.getByTestId("sidebar-community-brand-fallback"),
  ).toHaveText("C");
  await expect(brandMark.locator("img")).toHaveCount(0);
  await expect(brandMark.locator(".colony-sidebar-brand-dot")).toBeVisible();
});

test("header falls back to the saved business logo when the community icon fails", async ({
  page,
}) => {
  const brokenIcon = "https://icon.invalid/workspace-icon-unavailable.png";
  await page.route("**/workspace-icon-unavailable.png", (route) =>
    route.abort(),
  );
  await seedWorkspace(page, { icon: brokenIcon, logoUrl: BUSINESS_LOGO });
  await page.goto("/#/today");
  await openSidebar(page);

  const brandMark = page
    .getByTestId("sidebar-business-switcher")
    .getByTestId("sidebar-community-brand-mark");
  const fallbackLogo = brandMark.locator("img");
  await expect(fallbackLogo).toHaveAttribute("src", BUSINESS_LOGO);
  await expect(fallbackLogo).toBeVisible();
  await expect(brandMark).toHaveAttribute("data-source", "profile");
  await expect(brandMark.locator(".colony-sidebar-brand-dot")).toHaveCount(0);
});

test("returning-user business list prefers the cached community icon over its saved logo", async ({
  page,
}) => {
  await openReturningBusinessList(page, {
    holdCommunityIconFetch: true,
    includeProfileLogo: true,
  });
  const business = page.getByTestId(
    `onboarding-business-${SIGN_IN_BUSINESS_ID}`,
  );
  const mark = business.locator(".c3fr-mark");
  const image = mark.locator("img");
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(image).toBeVisible();
  await expect(mark).toHaveAttribute("data-source", "community");
  await expect(mark).toHaveText("");
  await page.evaluate(() => {
    type IconFetchWindow = Window & {
      __releaseWorkspaceIconFetch?: () => void;
    };
    (window as IconFetchWindow).__releaseWorkspaceIconFetch?.();
  });
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
});

test("returning-user business list falls back to its saved logo", async ({
  page,
}) => {
  await openReturningBusinessList(page, {
    communityIcon: null,
    includeProfileLogo: true,
  });
  const businessWithLogo = page.getByTestId(
    `onboarding-business-${SIGN_IN_BUSINESS_ID}`,
  );
  const logoMark = businessWithLogo.locator(".c3fr-mark");
  await expect(logoMark).toHaveAttribute("data-source", "profile");
  await expect(logoMark.locator("img")).toHaveAttribute("src", BUSINESS_LOGO);
  await expect(logoMark.locator("img")).toBeVisible();
});

test("returning-user business list shows the initial when no icon or logo exists", async ({
  page,
}) => {
  await openReturningBusinessList(page, { communityIcon: null });
  const business = page.getByTestId(
    `onboarding-business-${SIGN_IN_BUSINESS_ID}`,
  );
  const mark = business.locator(".c3fr-mark");
  await expect(mark).toHaveAttribute("data-source", "initial");
  await expect(mark.locator("img")).toHaveCount(0);
  await expect(mark).toHaveText("L");
});

test("Account settings shows the uploaded workspace icon", async ({ page }) => {
  await seedWorkspace(page, { icon: COMMUNITY_ICON });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/");
  await openSettings(page, "profile");

  const businessCard = page.getByTestId("settings-account-business-card");
  const mark = businessCard.locator(".cs-business-mark");
  const image = mark.locator("img");
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(image).toBeVisible();
  await expect(mark).toHaveAttribute("data-source", "community");
  await expect(mark).toHaveText("");
});

test("Business Profile shows the uploaded workspace icon", async ({ page }) => {
  await seedWorkspace(page, { icon: COMMUNITY_ICON });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/");
  await openSettings(page, "business-profile");

  const mark = page.getByTestId("business-profile-workspace-mark");
  const image = mark.locator("img");
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(image).toBeVisible();
  await expect(mark).toHaveAttribute("data-source", "community");
  await expect(mark).toHaveText("");
});

test("Company Map shows the uploaded workspace icon", async ({ page }) => {
  await seedWorkspace(page, {
    icon: COMMUNITY_ICON,
    mock: companyMapMock(),
  });
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/#/company/map");

  const mark = page.getByTestId("cofounder-map-company-logo");
  const image = mark.locator("img");
  await expect(page.getByTestId("cofounder-map-stage")).toBeVisible();
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(image).toBeVisible();
  await expect(mark).toHaveAttribute("data-source", "community");
  await expect(mark).toHaveText("");
});

test("Welcome Setup owned-business list shows the uploaded workspace icon", async ({
  page,
}) => {
  await openWelcomeOwnedBusinessList(page);

  const row = page
    .getByTestId(`welcome-owned-business-${OWNED_BUSINESS.id}`)
    .locator("xpath=..");
  const mark = row.locator(".c3fr-mark");
  const image = mark.locator("img");
  await expect(image).toHaveAttribute("src", COMMUNITY_ICON);
  await expect(image).toBeVisible();
  await expect(mark).toHaveAttribute("data-source", "community");
  await expect(mark).toHaveText("");
});

const LOGO_DEV_URL = "https://img.logo.dev/colony.example?token=e2e-test-token";
const LOGO_DEV_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" rx="12" fill="#68c8bf"/></svg>';

test("Account settings falls back to its saved website Logo.dev URL", async ({
  page,
}) => {
  await page.route("https://img.logo.dev/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: LOGO_DEV_SVG,
    }),
  );
  await seedWorkspace(page, {
    icon: null,
    logoUrl: LOGO_DEV_URL,
    website: "https://colony.example",
  });
  await page.goto("/");
  await openSettings(page, "profile");

  const mark = page
    .getByTestId("settings-account-business-card")
    .locator(".cs-business-mark");
  await expect(mark).toHaveAttribute("data-source", "profile");
  await expect(mark.locator("img")).toHaveAttribute("src", LOGO_DEV_URL);
  await expect(mark.locator("img")).toBeVisible();
});

test("Business Profile shows a letter only when no workspace icon or logo exists", async ({
  page,
}) => {
  await seedWorkspace(page);
  await page.goto("/");
  await openSettings(page, "business-profile");

  const mark = page.getByTestId("business-profile-workspace-mark");
  await expect(mark).toHaveAttribute("data-source", "initial");
  await expect(mark.locator("img")).toHaveCount(0);
  await expect(mark).toHaveText("C");
});

type MarkCaptureSurface =
  | "account"
  | "business-profile"
  | "company-map"
  | "welcome-owned-business";

const MARK_CAPTURE_VIEWS = [
  { key: "1440-light", width: 1440, height: 960, theme: "light" },
  { key: "1440-dark", width: 1440, height: 960, theme: "dark" },
  { key: "390-light", width: 390, height: 844, theme: "light" },
  { key: "390-dark", width: 390, height: 844, theme: "dark" },
] as const;

async function openMarkCaptureSurface(
  page: Page,
  surface: MarkCaptureSurface,
  view: (typeof MARK_CAPTURE_VIEWS)[number],
): Promise<Locator> {
  const viewport = { width: view.width, height: view.height };
  if (surface === "welcome-owned-business") {
    await openWelcomeOwnedBusinessList(page, {
      theme: view.theme,
      viewport,
    });
    return page
      .getByTestId(`welcome-owned-business-${OWNED_BUSINESS.id}`)
      .locator("xpath=..")
      .locator(".c3fr-mark");
  }

  await page.setViewportSize(viewport);
  await seedWorkspace(page, {
    icon: COMMUNITY_ICON,
    theme: view.theme,
    ...(surface === "company-map" ? { mock: companyMapMock() } : {}),
  });
  if (surface === "company-map") {
    await page.goto("/#/company/map");
    return page.getByTestId("cofounder-map-company-logo");
  }
  await page.goto(
    surface === "account"
      ? "/#/settings?section=profile"
      : "/#/settings?section=business-profile",
  );
  await expect(page.getByTestId("settings-view")).toBeVisible();
  return surface === "account"
    ? page
        .getByTestId("settings-account-business-card")
        .locator(".cs-business-mark")
    : page.getByTestId("business-profile-workspace-mark");
}

for (const surface of [
  "account",
  "business-profile",
  "company-map",
  "welcome-owned-business",
] as const) {
  for (const view of MARK_CAPTURE_VIEWS) {
    test(`captures fixed workspace mark: ${surface} ${view.key}`, async ({
      page,
    }) => {
      test.skip(
        !CAPTURE_DIR,
        "Set WS_HEADER_CAPTURE_DIR to write review captures.",
      );
      mkdirSync(CAPTURE_DIR as string, { recursive: true });
      const mark = await openMarkCaptureSurface(page, surface, view);
      await expect(mark.getByRole("img")).toHaveCount(0);
      await expect(mark.locator("img")).toHaveAttribute("src", COMMUNITY_ICON);
      await expect(mark.locator("img")).toBeVisible();
      await mark.scrollIntoViewIfNeeded();
      await waitForAnimations(page);
      await page.screenshot({
        fullPage: false,
        path: join(
          CAPTURE_DIR as string,
          `${CAPTURE_PHASE}-mark-${surface}-${view.key}.png`,
        ),
      });
    });
  }
}

const captureViews = [
  { key: "expanded-1440-light", width: 1440, height: 900, theme: "light" },
  { key: "expanded-1440-dark", width: 1440, height: 900, theme: "dark" },
  { key: "expanded-390-light", width: 390, height: 844, theme: "light" },
  { key: "expanded-390-dark", width: 390, height: 844, theme: "dark" },
  {
    key: "collapsed-1440-light",
    width: 1440,
    height: 900,
    theme: "light",
    collapsed: true,
  },
  {
    key: "collapsed-1440-dark",
    width: 1440,
    height: 900,
    theme: "dark",
    collapsed: true,
  },
] as const;

for (const view of captureViews) {
  test(`captures owner view: ${view.key}`, async ({ page }) => {
    test.skip(
      !CAPTURE_DIR,
      "Set WS_HEADER_CAPTURE_DIR to write review captures.",
    );
    mkdirSync(CAPTURE_DIR as string, { recursive: true });
    await page.setViewportSize({ width: view.width, height: view.height });
    await seedWorkspace(page, {
      icon: COMMUNITY_ICON,
      theme: view.theme,
    });
    await page.goto("/#/today");
    await openSidebar(page);

    await setDesktopSidebarCollapsed(
      page,
      "collapsed" in view && view.collapsed,
    );
    await waitForAnimations(page);
    await page.screenshot({
      fullPage: false,
      path: join(CAPTURE_DIR as string, `${CAPTURE_PHASE}-${view.key}.png`),
    });
  });
}

const signInCaptureViews = [
  {
    key: "signin-businesses-1440-light",
    width: 1440,
    height: 960,
    theme: "light",
  },
  {
    key: "signin-businesses-1440-dark",
    width: 1440,
    height: 960,
    theme: "dark",
  },
  {
    key: "signin-businesses-390-light",
    width: 390,
    height: 844,
    theme: "light",
  },
  { key: "signin-businesses-390-dark", width: 390, height: 844, theme: "dark" },
] as const;

for (const view of signInCaptureViews) {
  test(`captures owner view: ${view.key}`, async ({ page }) => {
    test.skip(
      !CAPTURE_DIR,
      "Set WS_HEADER_CAPTURE_DIR to write review captures.",
    );
    await openReturningBusinessList(page, {
      capture: view.key,
      theme: view.theme,
      viewport: { width: view.width, height: view.height },
    });
  });
}
