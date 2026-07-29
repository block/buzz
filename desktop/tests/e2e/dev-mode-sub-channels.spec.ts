import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Sub-channels (`parent--sub`, surfaced to users as "tabs") in developer
// mode: only main channels render in the left list; a parent's subs surface
// as tabs across the top of the open channel, and "+ tab" drafts a prompt
// whose Enter spawns a new sub-channel and announces it in the parent.

async function openDevModeChannel(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await composer.focus();

  const topBar = page.getByTestId("dev-mode-topbar-channel");
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();
}

async function createChannel(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.evaluate(async (channelName) => {
    const w = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string, payload: unknown) => Promise<unknown>;
      };
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    await w.__TAURI_INTERNALS__?.invoke("create_channel", {
      name: channelName,
      channelType: "stream",
      visibility: "open",
    });
    await w.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, name);
}

test("subs hide from the left list and surface as tabs in the parent", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");
  await createChannel(page, "general--flaky-ci");
  await createChannel(page, "general--rollback-plan");

  // The left list shows the parent but neither sub.
  const navigator = page.getByTestId("dev-mode-channel-navigator");
  await expect(navigator.getByText("# general", { exact: true })).toBeVisible();
  await expect(navigator.getByText("# general--flaky-ci")).toHaveCount(0);

  // Tabs: main + one per sub, labeled by sub slug only.
  const tabs = page.getByTestId("dev-mode-channel-tab");
  await expect(tabs).toHaveCount(3);
  await expect(tabs.nth(0)).toHaveText("main");
  await expect(tabs.nth(1)).toHaveText("flaky-ci");
  await expect(tabs.nth(2)).toHaveText("rollback-plan");
  await expect(tabs.nth(0)).toHaveAttribute("data-active", "true");

  // Switching tabs moves the transcript/composer to the sub-channel.
  await tabs.nth(1).click();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).toContainText("general--flaky-ci");
  await expect(tabs.nth(1)).toHaveAttribute("data-active", "true");

  // Escape backs out to the navigator with the *parent* highlighted.
  await page.keyboard.press("Escape");
  await expect(navigator.getByText("# general", { exact: true })).toBeVisible();
  await expect(navigator.getByText("▸")).toBeVisible();
});

test("+ tab drafts a prompt that spawns and announces a sub-channel", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  await page.getByTestId("dev-mode-new-tab").click();
  const composer = page.getByTestId("dev-mode-composer");
  await expect(composer).toHaveAttribute(
    "placeholder",
    /spawns a new tab in # general/,
  );

  // The draft state gets an unmissable highlight banner, and Escape drops
  // both the banner and the draft.
  const banner = page.getByTestId("dev-mode-draft-banner");
  await expect(banner).toContainText("new tab in # general");
  await page.keyboard.press("Escape");
  await expect(banner).not.toBeVisible();

  // ⌘⇧T re-enters the draft.
  await page.keyboard.press("Meta+Shift+t");
  await expect(banner).toBeVisible();

  await composer.pressSequentially("investigate the flaky deploy step");
  await page.keyboard.press("Enter");

  // Landed on the new sub-channel's tab.
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).toContainText("general--");
  const activeTab = page.locator(
    "[data-testid='dev-mode-channel-tab'][data-active]",
  );
  await expect(activeTab).not.toHaveText("main");

  // The parent's main tab carries the spawn announcement.
  await page.getByTestId("dev-mode-channel-tab").first().click();
  await expect(topBar).toContainText("# general");
  await expect(
    page.getByTestId("dev-mode-transcript").getByText(/spawned #general--/),
  ).toBeVisible();
});

test("palette offers a new tab action inside a channel", async ({ page }) => {
  await openDevModeChannel(page, "general");

  await page.keyboard.press("Control+o");
  await page.getByTestId("dev-mode-palette-input").pressSequentially("new tab");
  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("new tab in # general");
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("dev-mode-palette")).not.toBeVisible();
  await expect(page.getByTestId("dev-mode-composer")).toHaveAttribute(
    "placeholder",
    /spawns a new tab in # general/,
  );
});

test("⌘[ and ⌘] cycle through a channel's tabs", async ({ page }) => {
  await openDevModeChannel(page, "general");
  await createChannel(page, "general--flaky-ci");
  await createChannel(page, "general--rollback-plan");

  const tabs = page.getByTestId("dev-mode-channel-tab");
  await expect(tabs).toHaveCount(3);
  await expect(tabs.nth(0)).toHaveAttribute("data-active", "true");

  await page.keyboard.press("Meta+]");
  await expect(tabs.nth(1)).toHaveAttribute("data-active", "true");
  await page.keyboard.press("Meta+]");
  await expect(tabs.nth(2)).toHaveAttribute("data-active", "true");
  // Wraps around past the last tab.
  await page.keyboard.press("Meta+]");
  await expect(tabs.nth(0)).toHaveAttribute("data-active", "true");
  // ⌘[ goes the other way (wrapping back to the end).
  await page.keyboard.press("Meta+[");
  await expect(tabs.nth(2)).toHaveAttribute("data-active", "true");
});
