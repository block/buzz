import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

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

/** Directly drive the relay client singleton state via the E2E test seam. */
async function driveConnectionDegraded(
  page: import("@playwright/test").Page,
  state: ConnectionState = "reconnecting",
) {
  if (state !== "connected") {
    // When driving to a degraded state, wait for the relay to reach "connected"
    // first. Without this guard, the mock relay's async auth handshake can race
    // the state override and write "connected" back over it.
    await page.waitForFunction(() => {
      const win = window as Window & {
        __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
        __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
      };
      return (
        typeof win.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function" &&
        typeof win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__ === "function" &&
        win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__() === "connected"
      );
    });
  } else {
    // When driving to "connected", just wait for the seam to be installed —
    // no need to gate on current state since we're overriding it directly.
    await page.waitForFunction(
      () =>
        typeof (
          window as Window & {
            __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
          }
        ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function",
    );
  }
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

test.describe("relay connectivity", () => {
  test("01 — sidebar unreachable card", async ({ page }) => {
    await installMockBridge(page, { channelsReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    // Wait for the E2E seam to be installed (bridge init is complete), then force
    // the relay into a degraded state. The card is gated on !isRelayConnectionConnected:
    // a stale channelsReadError alone (with state still "connected") no longer pins
    // the card — connected state is now authoritative so stale query errors don't
    // hold the UI open after an auto-reconnect.
    //
    // Use "reconnecting" (rather than "disconnected"): the card shows via
    // isRelayConnectionStateDegraded for reconnecting/stalled states, so the test
    // doesn't race the channels-query retry window. useRelayConnection debounces
    // non-healthy states by 2 s, so await with timeout 5 s.
    await page.waitForFunction(
      () =>
        typeof (
          window as Window & {
            __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
          }
        ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function",
    );
    await driveConnectionDegraded(page, "reconnecting");

    const relayCard = page.getByTestId("sidebar-relay-unreachable");
    await expect(relayCard).toBeVisible({
      timeout: 5_000,
    });
    await expect(relayCard).toContainText("Can't reach the relay");
    await expect(relayCard).toContainText("Click to connect");
    await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();
    await settle(page);

    // Clip to sidebar width (256px) so the card and channel list are both visible.
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
  });

  test("03 — canvas unreachable in management sheet", async ({ page }) => {
    await installMockBridge(page, { canvasReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
    await page.getByTestId("channel-canvas-ingress").click();

    // ChannelCanvas shows the destructive error paragraph in the drill-in view
    // when the query fails.
    const canvasSection = page.getByTestId("channel-canvas-section");
    await canvasSection.scrollIntoViewIfNeeded();
    await expect(
      canvasSection.getByText("Can't reach the relay."),
    ).toBeVisible();

    // Await Radix sheet animations before measuring the settled state.
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
  });

  test("05 — no-cache npub fallback when offline", async ({ page }) => {
    // No cache seeded — profile card falls back to the mock identity npub name.
    await installMockBridge(page, { profileReadError: RELAY_UNREACHABLE });
    await page.goto("/");

    const profileCard = page.getByTestId("sidebar-profile-card");
    // Default mock identity display name is "npub1mock...".
    await expect(profileCard).toContainText("npub1mock");
    await settle(page);
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
