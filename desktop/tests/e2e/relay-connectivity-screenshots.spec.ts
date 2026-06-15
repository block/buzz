import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/relay-connectivity-screenshots";
const RELAY_UNREACHABLE = "relay unreachable: connection refused";

// Minimal teal 8×8 PNG as a data URL — satisfies avatarDataUrl's data:image/ guard.
const MOCK_AVATAR_DATA_URL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAE0lEQVR4nGOQ2rrsPz7MMDIUAACluJ0BkoZ9dQAAAABJRU5ErkJggg==";

// Self-profile cache key: buzz-self-profile.v1:<relay>:<pubkey>
const MOCK_PUBKEY = "deadbeef".repeat(8);
const MOCK_RELAY_URL = "ws://localhost:3000";
const SELF_PROFILE_CACHE_KEY = `buzz-self-profile.v1:${MOCK_RELAY_URL}:${MOCK_PUBKEY}`;

async function settle(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.allSettled(document.getAnimations().map((a) => a.finished)),
  );
}

type ConnectionState =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stalled"
  | "disconnected";

/** Directly emit "reconnecting" on the relay client singleton via the E2E test seam. */
async function driveConnectionDegraded(
  page: import("@playwright/test").Page,
  state: ConnectionState = "reconnecting",
) {
  await page.evaluate((s) => {
    const setter = (
      window as Window & {
        __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: string) => void;
      }
    ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setter) throw new Error("E2E relay state setter not installed.");
    setter(s);
  }, state);
}

test.describe("relay connectivity screenshots", () => {
  test("01 — sidebar unreachable banner", async ({ page }) => {
    await installMockBridge(page, { channelsReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible();
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    await settle(page);

    // Clip to sidebar width (256px) so the banner and channel list are both visible.
    await page.screenshot({
      path: `${SHOTS}/01-sidebar-unreachable.png`,
      clip: { x: 0, y: 0, width: 256, height: 720 },
    });
  });

  test("02 — connection banner while reconnecting", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Wait for a healthy boot, then force the relay into "reconnecting" state.
    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionDegraded(page);

    // ConnectionBanner debounces non-healthy states by 2 s before rendering.
    await expect(page.getByTestId("connection-banner")).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.getByTestId("connection-banner-reconnect")).toBeVisible();
    await settle(page);

    // Capture a horizontal strip spanning the full width that shows the banner
    // above the content pane.
    await page.screenshot({
      path: `${SHOTS}/02-connection-banner.png`,
      clip: { x: 0, y: 0, width: 1280, height: 180 },
    });
  });

  test("03 — home feed unreachable", async ({ page }) => {
    await installMockBridge(page, { feedReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    // HomeView renders the error card when the feed query fails.
    await expect(
      page.getByText(
        "Can't reach the relay — check your VPN or network connection.",
      ),
    ).toBeVisible();
    await settle(page);

    await page.screenshot({
      path: `${SHOTS}/03-home-unreachable.png`,
      clip: { x: 0, y: 0, width: 1280, height: 500 },
    });
  });

  test("04 — canvas unreachable in management sheet", async ({ page }) => {
    await installMockBridge(page, { canvasReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-management-sheet")).toBeVisible();

    // ChannelCanvas shows the destructive error paragraph when the query fails.
    const canvasSection = page.getByTestId("channel-canvas-section");
    await canvasSection.scrollIntoViewIfNeeded();
    await expect(
      canvasSection.getByText("Can't reach the relay."),
    ).toBeVisible();

    // Await Radix sheet animations before screenshotting.
    const sheet = page.getByTestId("channel-management-sheet");
    await sheet.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );
    await settle(page);

    // Capture the whole sheet so the error renders in its Canvas-section context.
    await sheet.screenshot({
      path: `${SHOTS}/04-canvas-unreachable.png`,
    });
  });

  test("05 — cached identity shown offline (avatar + display name)", async ({
    page,
  }) => {
    // Seed the self-profile cache BEFORE installMockBridge so addInitScript
    // runs in order and the cache is present when React mounts.
    await page.addInitScript(
      ({ key, cache }) => {
        window.localStorage.setItem(key, JSON.stringify(cache));
      },
      {
        key: SELF_PROFILE_CACHE_KEY,
        cache: {
          version: 1,
          displayName: "Tyler Durden",
          avatarUrl: "http://localhost:3999/avatar.png",
          avatarDataUrl: MOCK_AVATAR_DATA_URL,
          updatedAt: 1_700_000_000_000,
        },
      },
    );
    await installMockBridge(page, { profileReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    // The profile card should show the cached display name.
    const profileCard = page.getByTestId("sidebar-profile-card");
    await expect(profileCard).toContainText("Tyler Durden");
    await settle(page);

    await profileCard.screenshot({
      path: `${SHOTS}/05-cached-identity-offline.png`,
    });
  });

  test("06 — no-cache npub fallback when offline", async ({ page }) => {
    // No cache seeded — profile card falls back to the mock identity npub name.
    await installMockBridge(page, { profileReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    const profileCard = page.getByTestId("sidebar-profile-card");
    // Default mock identity display name is "npub1mock...".
    await expect(profileCard).toContainText("npub1mock");
    await settle(page);

    await profileCard.screenshot({
      path: `${SHOTS}/06-no-cache-npub-fallback.png`,
    });
  });

  test("07 — profile popover reconnect button while degraded", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionDegraded(page);

    // 2 s debounce on the "reconnecting" state before ConnectionBanner shows.
    await expect(page.getByTestId("connection-banner")).toBeVisible({
      timeout: 5_000,
    });

    await page.getByTestId("sidebar-profile-avatar-button").click();
    const reconnectBtn = page.getByTestId("profile-popover-reconnect");
    await expect(reconnectBtn).toBeVisible();

    // Wait for the Radix popover open animation on the [data-state] ancestor —
    // the popper wrapper itself carries no animations, so querying it returns
    // an empty list and screenshots capture a half-faded popover.
    await reconnectBtn.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );

    // Clip to the sidebar-bottom region that includes the open popover.
    await page.screenshot({
      path: `${SHOTS}/07-profile-popover-reconnect.png`,
      clip: { x: 0, y: 300, width: 480, height: 420 },
    });
  });
});
