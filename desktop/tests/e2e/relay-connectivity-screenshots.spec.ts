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
    // Tolerate cancelled animations: a SkeletonReveal animation cancelled
    // mid-flight (skeleton → live content swap) rejects `.finished` with an
    // AbortError. allSettled lets the animations that DO finish settle instead
    // of aborting the whole wait on the first cancel.
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
  test("01 — sidebar unreachable card", async ({ page }) => {
    await installMockBridge(page, { channelsReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    const relayCard = page.getByTestId("sidebar-relay-unreachable");
    await expect(relayCard).toBeVisible();
    await expect(relayCard).toContainText("Can't reach the relay");
    await expect(relayCard).toContainText("Click to connect");
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    await expect(page.getByTestId("connection-banner")).toHaveCount(0);
    await settle(page);

    // Clip to sidebar width (256px) so the card and channel list are both visible.
    await page.screenshot({
      path: `${SHOTS}/01-sidebar-unreachable.png`,
      clip: { x: 0, y: 0, width: 256, height: 720 },
    });
  });

  test("02 — sidebar reconnect card while reconnecting", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Wait for a healthy boot, then force the relay into "reconnecting" state.
    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionDegraded(page);

    // useRelayConnection debounces non-healthy states by 2 s before surfacing.
    const relayCard = page.getByTestId("sidebar-relay-unreachable");
    await expect(relayCard).toBeVisible({
      timeout: 5_000,
    });
    await expect(relayCard).toContainText("Can't reach the relay");
    await expect(relayCard).toContainText("Click to connect");
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    await settle(page);

    // Clip to the sidebar, where degraded relay state is now surfaced.
    await page.screenshot({
      path: `${SHOTS}/02-sidebar-reconnecting.png`,
      clip: { x: 0, y: 0, width: 256, height: 720 },
    });
  });

  test("03 — canvas unreachable in management sheet", async ({ page }) => {
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
      path: `${SHOTS}/03-canvas-unreachable.png`,
    });
  });

  test("04 — cached identity shown offline (avatar + display name)", async ({
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
      path: `${SHOTS}/04-cached-identity-offline.png`,
    });
  });

  test("05 — no-cache npub fallback when offline", async ({ page }) => {
    // No cache seeded — profile card falls back to the mock identity npub name.
    await installMockBridge(page, { profileReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    const profileCard = page.getByTestId("sidebar-profile-card");
    // Default mock identity display name is "npub1mock...".
    await expect(profileCard).toContainText("npub1mock");
    await settle(page);

    await profileCard.screenshot({
      path: `${SHOTS}/05-no-cache-npub-fallback.png`,
    });
  });

  test("06 — sidebar card shows connected after external relay recovery", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await expect(page.getByTestId("channel-general")).toBeVisible();
    await driveConnectionDegraded(page);

    const relayCard = page.getByTestId("sidebar-relay-unreachable");
    await expect(relayCard).toBeVisible({
      timeout: 5_000,
    });
    await expect(relayCard).toContainText("Can't reach the relay");
    await expect(relayCard).toContainText("Click to connect");

    await driveConnectionDegraded(page, "connected");

    await expect(relayCard).toContainText("Connected");
    await expect(relayCard).not.toContainText("Click to connect");
    await page.waitForTimeout(3_000);
    await expect(relayCard).toContainText("Connected");
    await expect(relayCard).toBeHidden({
      timeout: 5_000,
    });
  });
});
