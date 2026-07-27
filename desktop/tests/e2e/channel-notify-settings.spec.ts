import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/channel-notify-settings";

// Mock-mode current-user pubkey and relay (see e2eBridge DEFAULT_MOCK_PUBKEY /
// DEFAULT_RELAY_WS_URL). NIP-CN prefs persist under the relay-scoped key
// buzz-channel-notify-prefs.v1:<encoded-relay>:<pubkey>.
const MOCK_PUBKEY = "deadbeef".repeat(8);
const MOCK_RELAY_ENCODED = encodeURIComponent("ws://localhost:3000");
const PREFS_STORAGE_KEY = `buzz-channel-notify-prefs.v1:${MOCK_RELAY_ENCODED}:${MOCK_PUBKEY}`;
// `engineering` has no pre-seeded messages, so it holds clean visual states
// (`general` is always unread).
const ENGINEERING_CHANNEL_ID = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

const SIDEBAR_CLIP = { x: 0, y: 0, width: 256, height: 720 };
const MENU_CLIP = { x: 0, y: 0, width: 640, height: 720 };

function seedPrefs(
  page: Page,
  channelId: string,
  entry: Record<string, unknown>,
) {
  // addInitScript must run before installMockBridge: React reads the store on
  // mount and the bridge triggers that mount.
  return page.addInitScript(
    ({ key, channelId, entry }) => {
      window.localStorage.setItem(
        key,
        JSON.stringify({ version: 1, channels: { [channelId]: entry } }),
      );
    },
    { key: PREFS_STORAGE_KEY, channelId, entry },
  );
}

async function openApp(page: Page, activeChannel = "general") {
  await page.goto("/");
  await page.getByTestId(`channel-${activeChannel}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(activeChannel);
}

/** Right-click a sidebar channel and open its Notifications submenu. */
async function openNotificationsSubmenu(page: Page, channelName: string) {
  await page.getByTestId(`channel-${channelName}`).click({ button: "right" });
  const trigger = page.getByTestId("channel-notify-submenu");
  await expect(trigger).toBeVisible();
  await trigger.click();
  await expect(page.getByTestId("channel-notify-level-mentions")).toBeVisible();
}

async function setLevel(
  page: Page,
  channelName: string,
  level: "all" | "mentions" | "mute",
) {
  await openNotificationsSubmenu(page, channelName);
  await page.getByTestId(`channel-notify-level-${level}`).click();
  // Selecting a level closes the menu; the mutation runs after that (the menu
  // helpers defer it so Radix can finish its close animation first).
  await expect(page.getByTestId("channel-notify-submenu")).toHaveCount(0);
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      ),
    )
    .toBe(true);
}

function emitMention(page: Page, channelName: string) {
  return page.evaluate(
    ({ channelName, pubkey, mockPubkey }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
            mentionPubkeys: string[];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content: "Ping — can you take a look?",
        pubkey,
        mentionPubkeys: [mockPubkey],
      });
    },
    {
      channelName,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mockPubkey: MOCK_PUBKEY,
    },
  );
}

test.describe("per-channel notification settings", () => {
  test("01 — Notifications submenu shows the level radio group", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openApp(page);

    await openNotificationsSubmenu(page, "engineering");
    await expect(page.getByTestId("channel-notify-level-all")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(page.getByTestId("channel-notify-mute-1-hour")).toBeVisible();

    await waitForAnimations(page);
    await page.screenshot({
      clip: MENU_CLIP,
      path: `${SHOTS}/01-notifications-submenu.png`,
    });
  });

  test("02 — Just mentions is checked and named in the header", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openApp(page);

    await setLevel(page, "engineering", "mentions");

    await page.getByTestId("channel-engineering").click();
    await expect(page.getByTestId("chat-title")).toHaveText("engineering");
    // The header description is the title tooltip on the channel name.
    await expect(page.getByTestId("chat-title")).toHaveAttribute(
      "title",
      /Notifications: Just mentions/,
    );

    await openNotificationsSubmenu(page, "engineering");
    await expect(
      page.getByTestId("channel-notify-level-mentions"),
    ).toHaveAttribute("aria-checked", "true");

    await waitForAnimations(page);
    await page.screenshot({
      clip: MENU_CLIP,
      path: `${SHOTS}/02-just-mentions.png`,
    });
  });

  test("03 — Mute and hide removes the channel from the sidebar", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openApp(page);

    await expect(page.getByTestId("channel-engineering")).toBeVisible();
    await setLevel(page, "engineering", "mute");
    await expect(page.getByTestId("channel-engineering")).toHaveCount(0);

    await waitForAnimations(page);
    await page.screenshot({
      clip: SIDEBAR_CLIP,
      path: `${SHOTS}/03-mute-and-hide.png`,
    });
  });

  test("04 — a mention resurfaces a hidden channel", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Subscribe to engineering first (live messages are dropped without a
    // subscription), then move away so the unread indicator can appear.
    await page.getByTestId("channel-engineering").click();
    await expect(page.getByTestId("chat-title")).toHaveText("engineering");
    await waitForMockLiveSubscription(page, "engineering");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    await setLevel(page, "engineering", "mute");
    await expect(page.getByTestId("channel-engineering")).toHaveCount(0);

    await emitMention(page, "engineering");
    const row = page.getByTestId("channel-engineering");
    await expect(row).toBeVisible();
    await expect(row.locator("svg.lucide-bell-off")).toHaveCount(1);

    await waitForAnimations(page);
    await page.screenshot({
      clip: SIDEBAR_CLIP,
      path: `${SHOTS}/04-hidden-channel-mention.png`,
    });
  });

  test("05 — a running timed mute shows its expiry and Unmute", async ({
    page,
  }) => {
    await seedPrefs(page, ENGINEERING_CHANNEL_ID, {
      muteUntil: Math.floor(Date.now() / 1_000) + 3_600,
      updatedAt: Math.floor(Date.now() / 1_000),
    });
    await installMockBridge(page);
    await openApp(page);

    await openNotificationsSubmenu(page, "engineering");
    await expect(page.getByText(/^Muted until /)).toBeVisible();
    await expect(page.getByTestId("channel-notify-unmute")).toBeVisible();
    // A timed mute is an overlay, not a level: no radio item is selected.
    await expect(page.getByTestId("channel-notify-level-mute")).toHaveAttribute(
      "aria-checked",
      "false",
    );

    await waitForAnimations(page);
    await page.screenshot({
      clip: MENU_CLIP,
      path: `${SHOTS}/05-timed-mute.png`,
    });
  });

  test("06 — the channel sheet exposes the notifications section", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openApp(page, "engineering");

    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-management-sheet")).toBeVisible();

    const section = page.getByTestId("channel-notifications-section");
    await section.scrollIntoViewIfNeeded();
    await expect(
      page.getByTestId("channel-notifications-desktop-toggle"),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-notifications-broadcasts-toggle"),
    ).toBeVisible();
    await expect(
      page.getByTestId("channel-notifications-edit-defaults"),
    ).toBeVisible();

    await waitForAnimations(page);
    await section.screenshot({
      path: `${SHOTS}/06-sheet-notifications-section.png`,
    });
  });
});
