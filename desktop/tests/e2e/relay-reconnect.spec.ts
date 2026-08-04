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

async function restartMockWebsockets(page: import("@playwright/test").Page) {
  const restarted = await page.evaluate(() => {
    const restart = (
      window as Window & {
        __BUZZ_E2E_RESTART_MOCK_WEBSOCKETS__?: () => number;
      }
    ).__BUZZ_E2E_RESTART_MOCK_WEBSOCKETS__;
    if (!restart)
      throw new Error("E2E websocket restart seam is not installed.");
    return restart();
  });
  expect(restarted).toBeGreaterThan(0);
}

async function setMockWebsocketUnavailable(
  page: import("@playwright/test").Page,
  unavailable: boolean,
) {
  await page.evaluate((value) => {
    const setUnavailable = window.__BUZZ_E2E_SET_MOCK_WEBSOCKET_UNAVAILABLE__;
    if (!setUnavailable) {
      throw new Error("E2E websocket availability seam is not installed.");
    }
    setUnavailable(value);
  }, unavailable);
}

async function activateRelayRateLimit(
  page: import("@playwright/test").Page,
  seconds: number,
) {
  await page.evaluate((duration) => {
    const activate = window.__BUZZ_E2E_ACTIVATE_RELAY_RATE_LIMIT__;
    if (!activate) {
      throw new Error("E2E relay rate-limit seam is not installed.");
    }
    activate(duration);
  }, seconds);
}

async function getMockWebsocketConnectAttempts(
  page: import("@playwright/test").Page,
) {
  return page.evaluate(() => {
    const getAttempts = window.__BUZZ_E2E_GET_WEBSOCKET_CONNECT_ATTEMPTS__;
    if (!getAttempts) {
      throw new Error("E2E websocket attempt seam is not installed.");
    }
    return getAttempts();
  });
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

async function queueAuthResponses(
  page: import("@playwright/test").Page,
  responses: Array<{ success: boolean; message: string }>,
) {
  await page.evaluate((queued) => {
    const queue = window.__BUZZ_E2E_QUEUE_AUTH_RESPONSES__;
    if (!queue) throw new Error("E2E AUTH response seam is not installed.");
    queue(queued);
  }, responses);
}

async function closeLiveSubscriptions(
  page: import("@playwright/test").Page,
  reason: string,
) {
  const closed = await page.evaluate((message) => {
    const close = window.__BUZZ_E2E_CLOSE_LIVE_SUBSCRIPTIONS__;
    if (!close) throw new Error("E2E live CLOSED seam is not installed.");
    return close(message);
  }, reason);
  expect(closed).toBeGreaterThan(0);
}

async function driveConnectionDegraded(
  page: import("@playwright/test").Page,
  state: "connected" | "reconnecting" | "stalled" | "disconnected",
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

test("failed initial relay dial retries automatically", async ({ page }) => {
  await installMockBridge(page, {
    websocketConnectErrors: ["mock relay pod unavailable"],
  });
  await page.goto("/");

  // App-shell preconnect owns a keep-alive request. The first native dial is
  // rejected before a socket ID exists; the session must still enter its
  // backoff loop and recover without a click, query, or reload.
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const getState = (
            window as Window & {
              __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
            }
          ).__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__;
          if (!getState) throw new Error("Relay state seam is not installed.");
          return getState();
        }),
      { timeout: 10_000 },
    )
    .toBe("connected");
  await expect(page.getByTestId("channel-general")).toBeVisible();
});

test("routine traffic cannot bypass outage backoff and recovery stays automatic", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();

  await setMockWebsocketUnavailable(page, true);
  await disconnectMockWebsockets(page);

  // Exercise the production query path throughout the outage. Before the
  // coordinator fix, each rejected query called ensureConnected(), cancelled
  // the scheduled timer, and dialed immediately. The fixed session keeps these
  // callers behind its single jittered exponential-backoff attempt.
  await page.evaluate(async () => {
    const deadline = Date.now() + 4_200;
    while (Date.now() < deadline) {
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
      await new Promise((resolve) => window.setTimeout(resolve, 100));
    }
  });

  const attempts = await getMockWebsocketConnectAttempts(page);
  expect(attempts.length).toBeGreaterThanOrEqual(2);
  expect(attempts.length).toBeLessThanOrEqual(3);
  for (let index = 1; index < attempts.length; index += 1) {
    expect(attempts[index] - attempts[index - 1]).toBeGreaterThanOrEqual(700);
  }

  await setMockWebsocketUnavailable(page, false);
  await expect
    .poll(
      () =>
        page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
      { timeout: 10_000 },
    )
    .toBe("connected");

  const afterRecovery = `automatic outage recovery ${Date.now()}`;
  await emitMockMessages(page, [
    { content: afterRecovery, createdAt: Math.floor(Date.now() / 1_000) },
  ]);
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-timeline")).toContainText(
    afterRecovery,
  );
});

test("authenticated reconnect reports connected while replay is rate-limited", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();

  await activateRelayRateLimit(page, 5);
  await disconnectMockWebsockets(page);

  // Replay remains intentionally blocked behind admission control, but socket
  // open + successful AUTH is already a healthy connection. The UI must not
  // claim the relay is unreachable for the rest of the gate window.
  await expect
    .poll(
      () =>
        page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
      { timeout: 3_000 },
    )
    .toBe("connected");
  await expect(page.getByTestId("sidebar-relay-unreachable")).toHaveCount(0);
});

test("service restart close resets accumulated backoff", async ({ page }) => {
  await installMockBridge(page, {
    websocketConnectErrors: ["down 1", "down 2", "down 3"],
  });
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible({
    timeout: 15_000,
  });

  const startedAt = Date.now();
  await restartMockWebsockets(page);
  await expect
    .poll(
      () =>
        page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
      { timeout: 2_500 },
    )
    .toBe("connected");
  expect(Date.now() - startedAt).toBeLessThan(2_500);
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
  // focused on profile/settings/community actions even while the relay is down.
  await driveConnectionDegraded(page, "stalled");
  await expect(page.getByTestId("sidebar-relay-unreachable")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("profile-popover-reconnect")).toHaveCount(0);
});

test("resume event short-circuits accumulated reconnect backoff", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();

  await setMockWebsocketUnavailable(page, true);
  await disconnectMockWebsockets(page);
  await expect
    .poll(() => getMockWebsocketConnectAttempts(page), { timeout: 10_000 })
    .toHaveLength(3);

  await setMockWebsocketUnavailable(page, false);
  const resumedAt = Date.now();
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect
    .poll(
      () =>
        page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
      {
        timeout: 2_000,
      },
    )
    .toBe("connected");
  expect(Date.now() - resumedAt).toBeLessThan(2_000);
});

test("resume events during repeated AUTH rejection cannot defeat the terminal cap", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Three consecutive rejections must latch terminal even when resume
  // events interleave with the handshakes: resume attempts bypass backoff
  // but must preserve the AUTH rejection streak (review finding on
  // PR #4737 — preconnect()'s streak reset previously made the cap
  // unreachable under focus/online bursts).
  //
  // Queue MORE rejections than the cap: superseded connection attempts
  // (resume racing the backoff timer) consume queue entries on handshakes
  // whose frames the client drops as stale, exactly like a real relay that
  // rejects every attempt. A queue of exactly 3 can be silently eaten and
  // a default-success AUTH would then reset the streak.
  await queueAuthResponses(
    page,
    Array.from({ length: 12 }, () => ({
      success: false,
      message: "auth-required: verification failed",
    })),
  );
  await disconnectMockWebsockets(page);

  // Fire resume events while the rejection sequence plays out. Repeated
  // dispatch (post-rate-limit spacing is irrelevant here: each poll tick
  // dispatches both events) guarantees at least one lands between
  // handshakes.
  await expect
    .poll(
      async () => {
        await page.evaluate(() => {
          window.dispatchEvent(new Event("online"));
          window.dispatchEvent(new Event("focus"));
        });
        return page.evaluate(() =>
          window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.(),
        );
      },
      { intervals: [500], timeout: 20_000 },
    )
    .toBe("disconnected");

  // Terminal is user-owned: further resume events must not revive the
  // session.
  await page.evaluate(() => {
    window.dispatchEvent(new Event("online"));
    window.dispatchEvent(new Event("focus"));
  });
  await page.waitForTimeout(1_000);
  expect(
    await page.evaluate(() =>
      window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.(),
    ),
  ).toBe("disconnected");
});

test("sub-2s degraded flap invalidates relay queries on recovery", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();

  await page.evaluate(() => {
    const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__ as
      | {
          invalidateQueries: (...args: unknown[]) => unknown;
          __rawHealInvalidations?: number;
        }
      | undefined;
    if (!queryClient)
      throw new Error("E2E query client seam is not installed.");
    const original = queryClient.invalidateQueries.bind(queryClient);
    queryClient.__rawHealInvalidations = 0;
    queryClient.invalidateQueries = (...args: unknown[]) => {
      queryClient.__rawHealInvalidations =
        (queryClient.__rawHealInvalidations ?? 0) + 1;
      return original(...args);
    };
  });

  await driveConnectionDegraded(page, "reconnecting");
  await page.waitForTimeout(100);
  await driveConnectionDegraded(page, "connected");

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
              __rawHealInvalidations?: number;
            }
          )?.__rawHealInvalidations ?? 0,
      ),
    )
    .toBeGreaterThan(0);
});

test("transient AUTH rejection reconnects and restores live traffic", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await queueAuthResponses(page, [
    { success: false, message: "auth-required: verification failed" },
    { success: true, message: "" },
  ]);
  await disconnectMockWebsockets(page);

  await expect
    .poll(
      () =>
        page.evaluate(() => window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.()),
      {
        timeout: 10_000,
      },
    )
    .toBe("connected");

  const recovered = `live after transient AUTH rejection ${Date.now()}`;
  await emitMockMessages(page, [
    { content: recovered, createdAt: Math.floor(Date.now() / 1_000) },
  ]);
  await expect(page.getByTestId("message-timeline")).toContainText(recovered);

  const sent = `send after transient AUTH rejection ${Date.now()}`;
  await page.getByTestId("message-input").fill(sent);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(sent);
});

test("auth-required CLOSED restores the active live subscription", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await closeLiveSubscriptions(page, "auth-required: not authenticated");

  const recovered = `live after auth-required CLOSED ${Date.now()}`;
  await expect
    .poll(
      async () => {
        await emitMockMessages(page, [
          { content: recovered, createdAt: Math.floor(Date.now() / 1_000) },
        ]);
        return page
          .getByTestId("message-timeline")
          .evaluate(
            (element, content) => (element.textContent ?? "").includes(content),
            recovered,
          );
      },
      { intervals: [1_100], timeout: 5_000 },
    )
    .toBe(true);
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

  // The newest backfilled message renders at the bottom once the reconnect
  // catch-up settles.
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("reconnect e2e missed 260", {
    timeout: 15_000,
  });

  // The virtualized timeline windows the oldest backfilled rows out of the DOM
  // while the user sits at the bottom. Walk upward with real wheel input: the
  // older-history sentinel arms on a genuine leave→enter transition, and each
  // prepend restores the visible anchor away from the top. Reaching "missed
  // 001" proves reconnect backfilled past the live-subscription limit.
  await timeline.hover();
  await expect
    .poll(
      async () => {
        // Compact same-author rows can leave the restored viewport inside the
        // sentinel band. Move down first so the next upward gesture creates the
        // leave→enter transition that arms another bounded page.
        await page.mouse.wheel(0, 1200);
        await page.waitForTimeout(40);
        await page.mouse.wheel(0, -6000);
        return timeline.evaluate((element) =>
          (element.textContent ?? "").includes("reconnect e2e missed 001"),
        );
      },
      { intervals: [100, 150, 250], timeout: 15_000 },
    )
    .toBe(true);
});
