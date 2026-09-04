import { expect, test, type Page, type Route } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Mocked-relay E2E for the Meetings (HiveTalk) feature.
 *
 * No real relay, no real HiveTalk, no LiveKit media server: every
 * `http://localhost:3000/meetings/*` request and the NIP-11 `/info`
 * capability probe is fulfilled by `page.route` stubs. The challenge-flow
 * signatures are produced by the mock bridge's `sign_event` handler.
 *
 * Covered: capability gating, the Meetings tab room list, the channel
 * "Start meeting" deep-link button, and the full
 * register -> 402 -> subscribe -> invoice -> settle -> auto-retry -> call
 * flow.
 */

const RELAY_HTTP = "http://localhost:3000";

/**
 * A LiveKit JWT whose payload carries `owner: true`, so `CallView` renders the
 * host controls. Signature is nonsense — the client decodes these claims for UI
 * gating only, and HiveTalk enforces the real thing server-side.
 */
const HOST_TOKEN = `e2e.${Buffer.from(
  JSON.stringify({ owner: true, video: { room: "stale-probe" } }),
)
  .toString("base64url")
  .replace(/=+$/, "")}.signature`;

const MODERATION_PATHS = new Set([
  "/kick-user",
  "/mute-user",
  "/room/notify-lock",
  "/room/mute-on-join",
]);

type MeetingsInfo = {
  meetings?: { provider?: string; proxy?: string; api_base?: string };
  supported_extensions?: string[];
};

const CAPABLE_INFO: MeetingsInfo = {
  meetings: {
    provider: "hivetalk",
    proxy: "/meetings",
    api_base: "https://premrelay.exe.xyz",
  },
  supported_extensions: ["buzz-meetings"],
};

async function stubInfo(page: Page, info: MeetingsInfo): Promise<void> {
  await page.route(`${RELAY_HTTP}/info`, (route) =>
    route.fulfill({
      body: JSON.stringify(info),
      contentType: "application/nostr+json",
    }),
  );
}

type ActiveRoom = { name: string; numParticipants?: number };

type SubscriptionStub = {
  status: string;
  entitled: boolean;
  room_quota: number;
  rooms_in_use: number;
  free_quota: number;
  grace_days: number;
  in_grace: boolean;
  paid_until: string;
  plan: string;
  can_record: boolean;
  pubkey: string;
};

type MeetingsStubOverrides = {
  rooms?: ActiveRoom[];
  /** Skip the 402 on the first register-room — for flows that aren't testing it. */
  registerAlwaysSucceeds?: boolean;
  /** Body for `/subscription`; omitted means 402 `subscription_required`. */
  subscription?: SubscriptionStub;
};

type ModerationCall = { path: string; body: unknown };

type MeetingsStubState = {
  rooms: ActiveRoom[];
  /** Raw bodies seen on the moderation endpoints, in order. */
  moderation: ModerationCall[];
  /** register-room attempts seen so far. First attempt 402s, later ones 200. */
  registerAttempts: number;
  /** payment/status polls seen so far. First poll pending, later ones settled. */
  paymentPolls: number;
  /** get-token requests seen so far — proves the call view actually reached the
   * token fetch rather than just rendering its loading shell. */
  getTokenAttempts: number;
};

/**
 * Stub every `/meetings/*` proxy endpoint. Returns the mutable state object so a
 * test can pre-seed the room list; counters drive the 402->200 and
 * pending->settled transitions across the flow.
 */
async function stubMeetings(
  page: Page,
  overrides: MeetingsStubOverrides = {},
): Promise<MeetingsStubState> {
  const state: MeetingsStubState = {
    rooms: overrides.rooms ?? [],
    moderation: [],
    registerAttempts: 0,
    paymentPolls: 0,
    getTokenAttempts: 0,
  };

  const json = (route: Route, body: unknown, status = 200) =>
    route.fulfill({
      status,
      body: JSON.stringify(body),
      contentType: "application/json",
    });

  await page.route(`${RELAY_HTTP}/meetings/**`, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace(/^\/meetings/, "");

    switch (path) {
      case "/auth/challenge":
        return json(route, {
          challenge: "e2e-challenge-jwt",
          nonce: `nonce-${Math.random().toString(36).slice(2)}`,
          expires_at: new Date(Date.now() + 5 * 60_000).toISOString(),
          domain: "premrelay.exe.xyz",
        });

      case "/plans":
        return json(route, [
          {
            plan: "bulk10_1y",
            title: "Bulk 10 · 1 year",
            amount_sats: 21_000,
            room_quota: 10,
            can_record: true,
            period: "year",
          },
        ]);

      case "/list-rooms":
        return json(route, state.rooms);

      case "/rooms-by-pubkey":
        return json(route, []);

      case "/subscription":
        if (overrides.subscription) {
          return json(route, overrides.subscription);
        }
        // Quiet path for the header badge — no active subscription yet.
        return json(route, { reason: "subscription_required" }, 402);

      case "/subscribe":
        return json(route, {
          intent_id: "intent-e2e-1",
          plan: "bulk10_1y",
          amount_sats: 21_000,
          bolt11: "lnbc210u1p3e2etestinvoice0000",
          payment_hash: "hash-e2e-1",
          expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
          status: "pending",
        });

      case "/payment/status": {
        state.paymentPolls += 1;
        if (state.paymentPolls <= 1) {
          return json(route, { intent_id: "intent-e2e-1", status: "pending" });
        }
        return json(route, {
          intent_id: "intent-e2e-1",
          status: "settled",
          plan: "bulk10_1y",
          subscription: {
            status: "active",
            entitled: true,
            room_quota: 10,
            rooms_in_use: 0,
            free_quota: 0,
            grace_days: 3,
            in_grace: false,
            paid_until: new Date(Date.now() + 365 * 86_400_000).toISOString(),
            plan: "bulk10_1y",
            can_record: true,
            pubkey: "deadbeef".repeat(8),
          },
        });
      }

      case "/register-room": {
        state.registerAttempts += 1;
        if (!overrides.registerAlwaysSucceeds && state.registerAttempts <= 1) {
          return json(route, { reason: "subscription_required" }, 402);
        }
        const body = JSON.parse(route.request().postData() ?? "{}") as {
          room_name?: string;
        };
        // HiveTalk answers `400 room_name is required` to any other shape, so
        // the stub does too: no default, or a wrong production field name would
        // sail through this suite (it did — see MEETINGS_MODERATION_FIELD_SHAPE).
        const roomName = body.room_name;
        if (!roomName) {
          return json(route, { error: "room_name is required" }, 400);
        }
        return json(route, {
          room_id: "room-e2e-1",
          room_name: roomName,
          pubkey: "deadbeef".repeat(8),
        });
      }

      case "/get-token":
        state.getTokenAttempts += 1;
        return json(route, {
          token: HOST_TOKEN,
          url: "wss://livekit.invalid",
        });

      default:
        // Moderation endpoints: record the raw body so a test can assert the
        // exact wire shape HiveTalk's openapi.yaml requires.
        if (MODERATION_PATHS.has(path)) {
          state.moderation.push({
            path,
            body: JSON.parse(route.request().postData() ?? "null"),
          });
          return json(route, { ok: true });
        }
        return json(route, { error: `unstubbed meetings path: ${path}` }, 500);
    }
  });

  return state;
}

test("relay without the buzz-meetings capability hides Meetings", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, { supported_extensions: [] });
  await stubMeetings(page);

  await page.goto("/#/meetings");
  await expect(page.getByTestId("meetings-unavailable")).toBeVisible();

  await page.goto("/");
  // Anchor on a sibling nav item first: `toHaveCount(0)` passes trivially
  // against a sidebar that hasn't rendered yet.
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  // The sidebar entry is part of "hides Meetings" — the assertion this test was
  // missing while the entry shipped visible on incapable relays.
  await expect(page.getByTestId("open-meetings-view")).toHaveCount(0);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByRole("button", { name: "Start meeting" })).toHaveCount(
    0,
  );
});

test("relay with the buzz-meetings capability keeps the sidebar entry", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  await stubMeetings(page);

  await page.goto("/");
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  // Guards the other direction: the capability gate must not regress into
  // "always hidden."
  await expect(page.getByTestId("open-meetings-view")).toHaveCount(1);
});

test("Meetings tab lists the relay's active rooms", async ({ page }) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  await stubMeetings(page, {
    rooms: [
      { name: "design-sync", numParticipants: 3 },
      { name: "watercooler", numParticipants: 0 },
    ],
  });

  await page.goto("/#/meetings");

  const list = page.getByTestId("meeting-room-list");
  await expect(list).toBeVisible();
  await expect(list.getByText("design-sync")).toBeVisible();
  await expect(list.getByText("watercooler")).toBeVisible();
  await expect(list.getByText("Live")).toBeVisible();
});

test("channel Start meeting button deep-links into a prefilled start form", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  await stubMeetings(page);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByRole("button", { name: "Start meeting" }).click();

  await page.waitForURL(/#\/meetings\?/);
  expect(page.url()).toContain("action=start");

  const roomInput = page
    .getByTestId("start-meeting-form")
    .getByLabel("Room name");
  await expect(roomInput).toHaveValue(/^general-/);
  await expect(roomInput).toBeFocused();
});

test("register -> 402 -> subscribe -> settle -> auto-retry lands in the call view", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  const state = await stubMeetings(page);

  await page.goto("/#/meetings");

  const roomInput = page
    .getByTestId("start-meeting-form")
    .getByLabel("Room name");
  await roomInput.fill("e2e standup");
  await page.getByRole("button", { name: "Start meeting" }).click();

  // 402 subscription_required opens the subscribe dialog with the plan list.
  const dialog = page.getByTestId("meeting-subscribe-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("meeting-plan-card")).toBeVisible();

  await dialog.getByRole("button", { name: "Choose plan" }).click();

  // Invoice panel: BOLT11 string + live countdown, before payment settles.
  await expect(dialog.getByTestId("meeting-invoice-panel")).toBeVisible();
  await expect(dialog.getByTestId("meeting-invoice-countdown")).toBeVisible();
  await expect(dialog.getByText("lnbc210u1p3e2etestinvoice0000")).toBeVisible();

  // Poll #2 settles -> dialog closes -> register-room auto-retries (now 200)
  // -> navigate into the call view.
  await expect(page.getByTestId("meeting-call-view")).toBeVisible({
    timeout: 20_000,
  });
  await page.waitForURL(/#\/meetings\?.*action=join/);
  expect(state.registerAttempts).toBeGreaterThanOrEqual(2);

  // The room name reached the flow normalized ("e2e standup" -> "e2e-standup"),
  // not verbatim.
  expect(page.url()).toContain("room=e2e-standup");

  // Proves the call view actually reached the token fetch — the bare
  // `meeting-call-view` testid also renders in the loading/error shells, so on
  // its own it would pass even with a broken token path.
  await expect
    .poll(() => state.getTokenAttempts, { timeout: 20_000 })
    .toBeGreaterThanOrEqual(1);
});

test("a subscription renewal doesn't restart an earlier room", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  const state = await stubMeetings(page, {
    registerAlwaysSucceeds: true,
    // Near expiry so the badge offers "Renew" — the purchase path that has
    // nothing to do with any room.
    subscription: {
      status: "active",
      entitled: true,
      room_quota: 10,
      rooms_in_use: 1,
      free_quota: 0,
      grace_days: 3,
      in_grace: false,
      paid_until: new Date(Date.now() + 2 * 86_400_000).toISOString(),
      plan: "bulk10_1y",
      can_record: true,
      pubkey: "deadbeef".repeat(8),
    },
  });

  await page.goto("/#/meetings");

  // 1. Start a room and land in the call.
  const roomInput = page
    .getByTestId("start-meeting-form")
    .getByLabel("Room name");
  await roomInput.fill("stale probe");
  await page.getByRole("button", { name: "Start meeting" }).click();

  await expect(page.getByTestId("meeting-call-view")).toBeVisible({
    timeout: 20_000,
  });
  await page.waitForURL(/#\/meetings\?.*action=join/);
  expect(state.registerAttempts).toBe(1);

  // 2. Leave it. Same mounted route — only the search params change, which is
  // exactly why a ref can outlive the attempt.
  // Testid, not the role: LiveKit's own `lk-disconnect-button` is also named
  // "Leave" inside the call view.
  await page.getByTestId("meeting-leave").click();
  await page.waitForURL((url) => !url.hash.includes("action=join"));

  // 3. Renew the subscription — an unrelated purchase.
  const badge = page.getByTestId("meeting-subscription-badge");
  await expect(badge).toBeVisible();
  await badge.getByRole("button", { name: "Renew" }).click();

  const dialog = page.getByTestId("meeting-subscribe-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Choose plan" }).click();

  // 4. Payment settles and the dialog closes — with no side effects on rooms.
  await expect(dialog).toBeHidden({ timeout: 20_000 });
  await expect.poll(() => state.registerAttempts, { timeout: 5_000 }).toBe(1);
  expect(page.url()).not.toContain("action=join");
  expect(page.url()).not.toContain("room=stale-probe");
});

/**
 * The moderation field-shape contract, end to end through the real client.
 *
 * HiveTalk's openapi.yaml requires camelCase `RoomToggle` (`{roomName,
 * enabled}`) on these endpoints, even though the registry endpoints are
 * snake_case. The unit test pins the builder; this pins that the builder's
 * output survives the mutation, the relay client and `JSON.stringify` and
 * reaches the wire byte for byte — the proxy forwards raw bytes because
 * HiveTalk signs `sha256(rawBody)`, so nothing downstream can fix a bad shape.
 */
test("host controls send HiveTalk's camelCase moderation body", async ({
  page,
}) => {
  await installMockBridge(page);
  await stubInfo(page, CAPABLE_INFO);
  const state = await stubMeetings(page, { registerAlwaysSucceeds: true });

  await page.goto("/#/meetings");

  const roomInput = page
    .getByTestId("start-meeting-form")
    .getByLabel("Room name");
  await roomInput.fill("stale probe");
  await page.getByRole("button", { name: "Start meeting" }).click();

  await expect(page.getByTestId("meeting-call-view")).toBeVisible({
    timeout: 20_000,
  });

  // Anchor on the control existing before asserting anything about its effect.
  const hostControls = page.getByTestId("meeting-host-controls");
  await expect(hostControls).toBeVisible();
  await hostControls.click();
  await page.getByRole("menuitemcheckbox", { name: "Lock room" }).click();

  await expect.poll(() => state.moderation.length, { timeout: 10_000 }).toBe(1);
  expect(state.moderation[0]).toEqual({
    path: "/room/notify-lock",
    body: { roomName: "stale-probe", enabled: true },
  });
});
