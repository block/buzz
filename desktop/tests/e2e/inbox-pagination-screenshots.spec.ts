/**
 * Inbox cursor pagination — scroll-end loads older conversations.
 *
 * PR #5834 windowed the mentions feed per conversation, which fixed
 * starvation but capped the inbox at the newest N conversations. This
 * change adds an inclusive `until` cursor bounding each conversation's
 * latest activity (conv_latest): scroll-end fetches the next page of whole
 * conversations at-or-older than the oldest conversation in hand.
 *
 * The test seeds more standalone conversations than one page holds. The
 * first page cannot contain the oldest markers; scrolling to the bottom of
 * the inbox list must fetch older pages until they surface.
 *
 * Run (against an isolated relay):
 *   BUZZ_E2E_RELAY_URL=http://localhost:3030 pnpm build:e2e && \
 *     pnpm exec playwright test --project=integration \
 *       tests/e2e/inbox-pagination-screenshots.spec.ts
 * Output: test-results/inbox-pagination/
 */
import { expect, test } from "@playwright/test";
import { hexToBytes } from "@noble/hashes/utils.js";
import { finalizeEvent, type VerifiedEvent } from "nostr-tools/pure";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { scrollInboxListToText } from "../helpers/inboxScroll";
import { assertRelaySeeded } from "../helpers/seed";

const SHOTS = "test-results/inbox-pagination";
const RELAY_HTTP = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";

// uuid5("buzz.channel.general") — fixed id seeded by
// scripts/setup-desktop-test-data.sh.
const GENERAL_ID = "9f28288a-d724-587a-9709-92dc7f967110";

// One page is 50 events; every seeded mention is its own conversation, so
// 60 conversations guarantees the oldest ones start beyond the first page.
const CONVERSATION_COUNT = 60;

// Unique per run so reruns against a shared relay stay deterministic: the
// assertions only track this run's markers, never leftovers.
const RUN_ID = Math.random().toString(36).slice(2, 8);

async function publish(
  sender: keyof typeof TEST_IDENTITIES,
  channelId: string,
  content: string,
  createdAt: number,
): Promise<VerifiedEvent> {
  const event = finalizeEvent(
    {
      kind: 9,
      content,
      created_at: createdAt,
      tags: [
        ["h", channelId],
        ["p", TEST_IDENTITIES.tyler.pubkey],
      ],
    },
    hexToBytes(TEST_IDENTITIES[sender].privateKey),
  );
  const response = await fetch(`${RELAY_HTTP}/events`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Pubkey": event.pubkey },
    body: JSON.stringify(event),
  });
  if (!response.ok) {
    throw new Error(
      `POST /events failed (${response.status}): ${await response.text()}`,
    );
  }
  return event;
}

function marker(index: number) {
  return `paged mention ${index} [${RUN_ID}]`;
}

/**
 * Seed CONVERSATION_COUNT standalone mentions in #general — each one is its
 * own conversation (no thread tags), index 0 oldest.
 *
 * The relay rejects events drifting more than ±15 min from server time
 * (ingest.rs MAX_TIMESTAMP_DRIFT_SECS), so the timeline is compressed into
 * the last ~13 minutes: 10s spacing from −780s. Ordering matters, not span.
 */
async function seedPagedConversations() {
  const nowSecond = Math.floor(Date.now() / 1000);
  for (let i = 0; i < CONVERSATION_COUNT; i++) {
    await publish("bob", GENERAL_ID, marker(i), nowSecond - 780 + i * 10);
  }
}

test.describe("inbox cursor pagination", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeAll(async () => {
    test.setTimeout(120_000);
    await assertRelaySeeded();
    await seedPagedConversations();
  });

  test("scroll-end loads older conversations beyond the first page", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    await installRelayBridge(page, "tyler");
    await page.goto("/");
    const list = page.getByTestId("home-inbox-list");
    await expect(list).toBeVisible();

    // Page 1 (newest 50 conversations) holds the newest marker…
    await expect(list).toContainText(marker(CONVERSATION_COUNT - 1), {
      timeout: 15_000,
    });
    // …and cannot hold the oldest: it is beyond the first page's window.
    await expect(list).not.toContainText(marker(0));
    await page.screenshot({
      path: `${SHOTS}/01-before-first-page-only.png`,
      fullPage: true,
    });

    // Scroll toward the bottom until the oldest conversation pages in. Each
    // scroll-end triggers a cursor fetch; the step-scroll helper walks the
    // virtualized list a viewport at a time so mid-list rows render too.
    await scrollInboxListToText(page, list, marker(0));
    await expect(list).toContainText(marker(0));

    await page.screenshot({
      path: `${SHOTS}/02-after-scroll-loaded-older.png`,
      fullPage: true,
    });
  });
});
