import { expect, test } from "@playwright/test";
import { finalizeEvent } from "nostr-tools/pure";
import { hexToBytes } from "@noble/hashes/utils.js";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";

const isCi = Boolean(process.env.CI);
const relaySeedHookTimeoutMs = isCi ? 90_000 : 30_000;

const RELAY_HTTP_URL =
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";

// setup-desktop-test-data.sh: uuid5(NAMESPACE_DNS, "buzz.channel.dm.alice-tyler")
const ALICE_TYLER_DM_CHANNEL_ID = "5a9c064e-0411-5242-ae6b-0363ba99b8e6";

async function getLoggedNotifications(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_NOTIFICATIONS__?: Array<{
        body: string | null;
        title: string;
      }>;
    };

    return win.__BUZZ_E2E_NOTIFICATIONS__ ?? [];
  });
}

/**
 * Publishes a REAL signed DM message from alice through the relay ingest
 * path. Like every DM send in the product, it carries a recipient `p` tag
 * (see messageMentionPubkeys) — which is exactly what makes the event match
 * both the live DM subscription and the home-feed mention query.
 */
async function publishAliceDm(content: string) {
  const event = finalizeEvent(
    {
      kind: 9,
      content,
      tags: [
        ["h", ALICE_TYLER_DM_CHANNEL_ID],
        ["p", TEST_IDENTITIES.tyler.pubkey],
      ],
      created_at: Math.floor(Date.now() / 1000),
    },
    hexToBytes(TEST_IDENTITIES.alice.privateKey),
  );

  const response = await fetch(`${RELAY_HTTP_URL}/events`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Pubkey": event.pubkey,
    },
    body: JSON.stringify(event),
  });
  if (!response.ok) {
    throw new Error(
      `POST /events failed (${response.status}): ${await response.text()}`,
    );
  }
}

test.beforeAll(async () => {
  test.setTimeout(relaySeedHookTimeoutMs);
  await assertRelaySeeded();
});

test("an incoming DM produces exactly one desktop notification", async ({
  page,
}) => {
  await installRelayBridge(page, "tyler");

  // Deterministically wait for the home feed's initial mention query
  // (kinds 9/... + #p filter) to complete before publishing: the feed
  // dedupe-seen set must be initialized, otherwise the duplicate is
  // accidentally swallowed as "initial backlog" and the repro goes flaky.
  const feedInitialized = page.waitForResponse(
    (response) =>
      response.url().includes("/query") &&
      (response.request().postData() ?? "").includes('"#p"'),
    { timeout: 15_000 },
  );
  await page.goto("/");

  // Wait until the DM channel is loaded — live subscriptions (channel + home
  // feed mention) are established once the channel list resolves.
  await expect(page.getByTestId("dm-list")).toContainText("alice-tyler");
  await feedInitialized;
  // Small buffer for the feed effect (seen-set initialization) to run.
  await page.waitForTimeout(1_000);

  const message = `dm dedupe probe ${Date.now()}`;
  await publishAliceDm(message);

  // Wait for the DM toast to arrive.
  await expect
    .poll(async () => (await getLoggedNotifications(page)).length, {
      timeout: 15_000,
    })
    .toBeGreaterThan(0);

  // Give the duplicate (home-feed mention path) time to fire — it arrives via
  // the onLiveMention → feed refetch round trip, which lags the WS toast.
  await page.waitForTimeout(5_000);

  const notifications = await getLoggedNotifications(page);
  expect(
    notifications,
    `expected exactly one notification for a single DM, got: ${JSON.stringify(notifications)}`,
  ).toHaveLength(1);

  // The survivor must be the live WebSocket DM toast (titled with the DM
  // channel name), not the home-feed mention duplicate ("… mentioned you in …").
  expect(notifications[0].title).toBe("alice-tyler");
});
