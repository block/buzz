import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function setMockWebsocketSendsStalled(
  page: import("@playwright/test").Page,
  stall: boolean,
) {
  await page.evaluate((shouldStall) => {
    const setter = (
      window as Window & {
        __BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__?: (stall: boolean) => void;
      }
    ).__BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__;
    if (!setter) {
      throw new Error("E2E websocket stall setter is not installed.");
    }
    setter(shouldStall);
  }, stall);
}

async function disconnectMockWebsockets(page: import("@playwright/test").Page) {
  const disconnected = await page.evaluate(() => {
    const disconnect = (
      window as Window & {
        __BUZZ_E2E_DISCONNECT_MOCK_WEBSOCKETS__?: () => number;
      }
    ).__BUZZ_E2E_DISCONNECT_MOCK_WEBSOCKETS__;
    if (!disconnect) {
      throw new Error("E2E mock websocket disconnect seam is not installed.");
    }
    return disconnect();
  });

  expect(disconnected).toBeGreaterThan(0);
}

async function emitMockMessages(
  page: import("@playwright/test").Page,
  messages: Array<{ content: string; createdAt: number }>,
) {
  await page.evaluate((items) => {
    const emit = (
      window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          createdAt: number;
        }) => unknown;
      }
    ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) {
      throw new Error("E2E mock message emitter is not installed.");
    }

    for (const item of items) {
      emit({ channelName: "general", ...item });
    }
  }, messages);
}

async function driveConnectionDegraded(
  page: import("@playwright/test").Page,
  state: "reconnecting" | "stalled" | "disconnected",
) {
  await page.evaluate((s) => {
    const setter = (
      window as Window & {
        __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: string) => void;
      }
    ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setter) {
      throw new Error("E2E relay state setter is not installed.");
    }
    setter(s);
  }, state);
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("passive relay watchdog does not write while the websocket is half-open", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  await setMockWebsocketSendsStalled(page, true);

  // Wait longer than the old active-probe interval. If the watchdog still
  // writes probes, the mocked plugin send would never resolve and mark the
  // mock plugin mutex as wedged. Future reconnects would then be unable to
  // register, matching the tauri-plugin-websocket failure mode. The passive
  // watchdog should perform no writes of its own during this window.
  await page.waitForTimeout(22_000);

  await setMockWebsocketSendsStalled(page, false);
  const message = `recovered after passive idle ${Date.now()}`;
  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("sidebar reconnect prompt flips on live relay degradation without a query error", async ({
  page,
}) => {
  await page.goto("/");

  // Healthy boot: channels render from the mock bridge and the reconnect
  // prompt is absent (no query error, connection healthy).
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);

  // Drive ONLY the live connection state degraded — no channelsQuery error is
  // set. Pre-fix the block keyed off `channelsQuery.error` alone, so it stays
  // absent here; post-fix the dual signal surfaces it.
  await driveConnectionDegraded(page, "stalled");

  // `stalled` is debounced before surfacing, then React needs a render tick.
  await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("sidebar-reconnect")).toBeVisible();

  // The cached channel list stays visible alongside the prompt (layout intent:
  // surface the affordance, don't yank context).
  await expect(page.getByTestId("channel-general")).toBeVisible();
});

test("profile popover does not show relay reconnect controls", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByTestId("channel-general")).toBeVisible();
  await page.getByTestId("sidebar-profile-avatar-button").click();
  await expect(page.getByTestId("profile-popover-settings")).toBeVisible();
  await expect(page.getByTestId("profile-popover-reconnect")).toHaveCount(0);

  // The sidebar owns the relay reconnect affordance; the profile popover stays
  // focused on profile/settings/workspace actions even while the relay is down.
  await driveConnectionDegraded(page, "stalled");
  await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("profile-popover-reconnect")).toHaveCount(0);
});

test("reconnect backfills more missed channel messages than the live subscription limit", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const baseCreatedAt = Math.floor(Date.now() / 1_000) - 300;
  const seenBeforeDisconnect = "reconnect e2e seen before disconnect";
  await emitMockMessages(page, [
    { content: seenBeforeDisconnect, createdAt: baseCreatedAt },
  ]);
  await expect(page.getByTestId("message-timeline")).toContainText(
    seenBeforeDisconnect,
  );

  await disconnectMockWebsockets(page);

  const missedMessages = Array.from({ length: 260 }, (_, index) => ({
    content: `reconnect e2e missed ${String(index + 1).padStart(3, "0")}`,
    createdAt: baseCreatedAt + index + 1,
  }));
  await emitMockMessages(page, missedMessages);

  await expect(page.getByTestId("message-timeline")).toContainText(
    "reconnect e2e missed 001",
    { timeout: 15_000 },
  );
  await expect(page.getByTestId("message-timeline")).toContainText(
    "reconnect e2e missed 260",
  );
});
