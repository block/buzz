/**
 * Inbox feed windowing — starvation regression + before/after screenshots.
 *
 * The mentions feed used to be a flat "newest N events p-tagging me" window.
 * Clients group events into conversation rows *after* that cut, so one chatty
 * DM or thread could occupy every slot and starve all other conversations out
 * of the Inbox entirely. The relay now windows the mentions feed
 * per-conversation (crates/buzz-db/src/feed.rs build_mentions_query) when the
 * query carries `feed_types`.
 *
 * Both tests seed the same shape: one chatty DM (60 messages, newest) plus 6
 * standalone mentions in #general (older). The "before" test strips
 * `feed_types` from the outgoing /query — reproducing the flat generic-query
 * window production used before this change — and shows the starved inbox.
 * The "after" test uses the windowed feed path and shows every conversation
 * surviving.
 *
 * Run (against an isolated relay):
 *   BUZZ_E2E_RELAY_URL=http://localhost:3030 pnpm build:e2e && \
 *     pnpm exec playwright test --project=integration \
 *       tests/e2e/inbox-windowing-screenshots.spec.ts
 * Output: test-results/inbox-windowing/
 */
import { expect, test, type Page, type Route } from "@playwright/test";
import { hexToBytes } from "@noble/hashes/utils.js";
import { finalizeEvent, type VerifiedEvent } from "nostr-tools/pure";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { scrollInboxListToText } from "../helpers/inboxScroll";
import { assertRelaySeeded } from "../helpers/seed";

const SHOTS = "test-results/inbox-windowing";
const RELAY_HTTP = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";

// uuid5("buzz.channel.dm.alice-tyler") / uuid5("buzz.channel.general") —
// fixed ids seeded by scripts/setup-desktop-test-data.sh.
const DM_ID = "5a9c064e-0411-5242-ae6b-0363ba99b8e6";
const GENERAL_ID = "9f28288a-d724-587a-9709-92dc7f967110";

const CHATTY_COUNT = 60;
const STANDALONE_COUNT = 6;

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

function standaloneMarker(index: number) {
  return `standalone mention ${index} [${RUN_ID}]`;
}

/**
 * Seed the starvation shape: 6 standalone mentions in #general (older),
 * then a 60-message chatty DM burst (newest). A flat 50-event window is
 * entirely consumed by the burst.
 *
 * The relay rejects events drifting more than ±15 min from server time
 * (ingest.rs MAX_TIMESTAMP_DRIFT_SECS), so the whole timeline is compressed
 * into the last ~14 minutes: mentions at −840s spaced 10s, burst at −600s
 * spaced 5s. Ordering is what matters, not the span.
 */
async function seedStarvationShape() {
  const nowSecond = Math.floor(Date.now() / 1000);
  for (let i = 0; i < STANDALONE_COUNT; i++) {
    await publish(
      "bob",
      GENERAL_ID,
      standaloneMarker(i),
      nowSecond - 840 + i * 10,
    );
  }
  for (let i = 0; i < CHATTY_COUNT; i++) {
    await publish(
      "alice",
      DM_ID,
      `chatty dm update ${i} [${RUN_ID}]`,
      nowSecond - 600 + i * 5,
    );
  }
}

/**
 * Strip `feed_types` from outgoing /query filters — the pre-windowing wire
 * shape. The relay then serves the mention filter through the flat
 * generic-query path, which is exactly what production clients sent before
 * this change.
 */
async function forceFlatFeedWindow(page: Page) {
  await page.route(
    (url) => url.pathname.endsWith("/query"),
    async (route: Route) => {
      const request = route.request();
      let body: unknown;
      try {
        body = request.postDataJSON();
      } catch {
        await route.continue();
        return;
      }
      if (!Array.isArray(body)) {
        await route.continue();
        return;
      }
      const stripped = body.map((filter) => {
        if (filter && typeof filter === "object" && "feed_types" in filter) {
          const { feed_types: _dropped, ...rest } = filter as Record<
            string,
            unknown
          >;
          return rest;
        }
        return filter;
      });
      await route.continue({ postData: JSON.stringify(stripped) });
    },
  );
}

function getListPane(page: Page) {
  return page.getByTestId("home-inbox-list");
}

test.describe("inbox feed windowing", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeAll(async () => {
    test.setTimeout(120_000);
    await assertRelaySeeded();
    await seedStarvationShape();
  });

  test("before — flat window lets one chatty DM starve the inbox", async ({
    page,
  }) => {
    await forceFlatFeedWindow(page);
    await installRelayBridge(page, "tyler");
    await page.goto("/");
    const list = getListPane(page);
    await expect(list).toBeVisible();

    // The chatty DM survives as a single collapsed row…
    await expect(list).toContainText(`chatty dm update ${CHATTY_COUNT - 1}`, {
      timeout: 15_000,
    });
    // …but every standalone mention fell off the flat 50-event cliff.
    for (let i = 0; i < STANDALONE_COUNT; i++) {
      await expect(list).not.toContainText(standaloneMarker(i));
    }

    await page.screenshot({
      path: `${SHOTS}/01-before-flat-window-starved.png`,
      fullPage: true,
    });
  });

  test("after — per-conversation windowing keeps every conversation", async ({
    page,
  }) => {
    await installRelayBridge(page, "tyler");
    await page.goto("/");
    const list = getListPane(page);
    await expect(list).toBeVisible();

    // The chatty DM still shows (capped, collapsed to one row)…
    await expect(list).toContainText(`chatty dm update ${CHATTY_COUNT - 1}`, {
      timeout: 15_000,
    });
    // …and every standalone mention conversation survives — on the first
    // page when the relay holds few conversations, or reachable via
    // scroll-end cursor pagination when other (newer) conversations from
    // sibling specs share the relay. Either way, nothing is starved out.
    for (let i = 0; i < STANDALONE_COUNT; i++) {
      await scrollInboxListToText(page, list, standaloneMarker(i));
    }

    await page.screenshot({
      path: `${SHOTS}/02-after-windowed-conversations.png`,
      fullPage: true,
    });
  });
});
