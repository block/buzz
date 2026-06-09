import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind: k }) => {
          return (
            (
              window as Window & {
                __SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind?: number;
                }) => boolean;
              }
            ).__SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: currentChannelName,
              kind: k,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

async function getBadgeState(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const w = window as Window & {
      __SPROUT_E2E_APP_BADGE_STATE__?: string;
      __SPROUT_E2E_APP_BADGE_COUNT__?: number;
    };
    return {
      state: w.__SPROUT_E2E_APP_BADGE_STATE__ ?? "none",
      count: w.__SPROUT_E2E_APP_BADGE_COUNT__ ?? 0,
    };
  });
}

async function waitForBadgeState(
  page: import("@playwright/test").Page,
  expected: { state: string; count?: number },
) {
  await expect
    .poll(async () => getBadgeState(page), { timeout: 5_000 })
    .toEqual(
      expect.objectContaining({
        state: expected.state,
        ...(expected.count !== undefined ? { count: expected.count } : {}),
      }),
    );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("dot badge for regular message in inactive channel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");

  await page.evaluate(
    ({ pubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Regular message, no mention",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();
  await waitForBadgeState(page, { state: "dot" });
});

test("numeric badge for @mention in inactive channel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");

  await page.evaluate(
    ({ pubkey, mentionPubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Hey @tyler check this out",
        kind: 40002,
        pubkey,
        mentionPubkeys: [mentionPubkey],
      });
    },
    {
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkey: DEFAULT_MOCK_PUBKEY,
    },
  );

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();
  await waitForBadgeState(page, { state: "count", count: 1 });
});

test("numeric badge for DM message", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "alice-tyler");

  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "Hey, got a minute?",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("channel-unread-alice-tyler")).toBeVisible();
  await waitForBadgeState(page, { state: "count", count: 1 });
});

test("numeric badge for broadcast reply in inactive channel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");

  await page.evaluate(
    ({ pubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Broadcast reply to the channel",
        kind: 40002,
        pubkey,
        extraTags: [
          ["broadcast", "1"],
          ["e", "some-root-event-id"],
        ],
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();
  await waitForBadgeState(page, { state: "count", count: 1 });
});

test("mark-as-read via context menu clears channel unread indicator", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "random");

  // Wait for catch-up to settle, then record baseline badge state
  // (other mock channels may have pre-existing unreads from seeded history)
  await page.waitForTimeout(2000);
  const baselineBadge = await getBadgeState(page);

  await page.evaluate(
    ({ pubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Message to be marked read",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark as read").click();

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await waitForBadgeState(page, { state: baselineBadge.state });
});

test("mark-as-unread via context menu shows dot badge", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark unread").click();

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();
  await waitForBadgeState(page, { state: "dot" });
});

test("remote read-state rollback is ignored while local mark-unread still shows dot", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Baseline: random has no unread dot
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

  // Wait for ReadStateManager's live subscription (kind:30078) to be
  // established before injecting events.
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        return (
          (
            window as Window & {
              __SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
                kind?: number;
              }) => boolean;
            }
          ).__SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
            kind: 30078,
          }) ?? false
        );
      });
    })
    .toBe(true);

  const REMOTE_CLIENT_ID = "other-device-client-id";
  const REMOTE_SLOT_ID = "e2e00000000000000000000000000000";
  const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
  const now = Math.floor(Date.now() / 1000);

  // Step 1: seed a "read at now" state from the remote device so the
  // local manager has a baseline value for this channel context.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts }) => {
      (
        window as Window & {
          __SPROUT_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__SPROUT_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt: ts,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now,
    },
  );

  // Step 2: a remote rollback carries an older read timestamp in a newer
  // event. NIP-RS read markers are monotonic, so this must be ignored.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts, createdAt }) => {
      (
        window as Window & {
          __SPROUT_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__SPROUT_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now - 100,
      createdAt: now + 5,
    },
  );

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

  // Local mark-unread remains an in-session affordance and should still show
  // the dot immediately without publishing a lower read timestamp.
  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByText("Mark unread").click();
  await expect(page.getByTestId("channel-unread-random")).toBeVisible();
  await waitForBadgeState(page, { state: "dot" });

  // Step 3: remote advance clears the local forced-unread dot.
  await page.evaluate(
    ({ clientId, slotId, channelId, ts, createdAt }) => {
      (
        window as Window & {
          __SPROUT_E2E_EMIT_MOCK_READ_STATE__?: (input: {
            clientId: string;
            contexts: Record<string, number>;
            createdAt: number;
            slotId: string;
          }) => unknown;
        }
      ).__SPROUT_E2E_EMIT_MOCK_READ_STATE__?.({
        clientId,
        slotId,
        contexts: { [channelId]: ts },
        createdAt,
      });
    },
    {
      clientId: REMOTE_CLIENT_ID,
      slotId: REMOTE_SLOT_ID,
      channelId: RANDOM_CHANNEL_ID,
      ts: now + 10,
      createdAt: now + 10,
    },
  );

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
});
