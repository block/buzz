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

type MeetingsStubState = {
  rooms: ActiveRoom[];
  /** register-room attempts seen so far. First attempt 402s, later ones 200. */
  registerAttempts: number;
  /** payment/status polls seen so far. First poll pending, later ones settled. */
  paymentPolls: number;
};

/**
 * Stub every `/meetings/*` proxy endpoint. Returns the mutable state object so a
 * test can pre-seed the room list; counters drive the 402->200 and
 * pending->settled transitions across the flow.
 */
async function stubMeetings(
  page: Page,
  overrides: Partial<MeetingsStubState> = {},
): Promise<MeetingsStubState> {
  const state: MeetingsStubState = {
    rooms: overrides.rooms ?? [],
    registerAttempts: 0,
    paymentPolls: 0,
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
        if (state.registerAttempts <= 1) {
          return json(route, { reason: "subscription_required" }, 402);
        }
        const body = JSON.parse(route.request().postData() ?? "{}") as {
          roomName?: string;
        };
        const roomName = body.roomName ?? "e2e-standup";
        return json(route, {
          room_id: "room-e2e-1",
          room_name: roomName,
          pubkey: "deadbeef".repeat(8),
        });
      }

      case "/get-token":
        return json(route, {
          token: "e2e.header.signature",
          url: "wss://livekit.invalid",
        });

      default:
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
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByRole("button", { name: "Start meeting" })).toHaveCount(
    0,
  );
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
});
