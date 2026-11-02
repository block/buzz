import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { expectUnreadBadgeCount } from "../helpers/unreadBadge";
import type { RelayEvent } from "../../src/shared/api/types";

/**
 * The relay wakes Scout with a kind 9 message in the DM between Scout and its
 * owner. The message is for Scout's harness, never for a person: it must not
 * show in the conversation, the sidebar unread count, Activity or the DM
 * header. It is recognised by the `buzz:cofounder-wake` tag AND the relay's own
 * signature, so a member copying the tag cannot make a message invisible.
 */

const RELAY_SELF = "ab".repeat(32);
const WAKE_TEXT =
  "Wake from the relay (co-founder mode): Co-founder mode was just switched on.";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

type MockWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    pubkey?: string;
    kind?: number;
    extraTags?: string[][];
    id?: string;
  }) => RelayEvent;
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
    category: "mention" | "needs_action" | "activity" | "agent_activity";
    channel_id: string | null;
    channel_name: string;
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  }) => unknown;
};

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false,
        channelName,
      ),
    )
    .toBe(true);
}

async function emitWake(page: Page, author: string) {
  await page.evaluate(
    ({ author, text }) => {
      (window as MockWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        content: text,
        pubkey: author,
        kind: 9,
        extraTags: [
          ["buzz:workflow", "true"],
          ["buzz:cofounder-wake", "mode_on"],
        ],
      });
    },
    { author, text: WAKE_TEXT },
  );
}

async function emitPlain(page: Page, content: string) {
  await page.evaluate(
    ({ content, pubkey }) => {
      (window as MockWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        content,
        pubkey,
        kind: 40002,
      });
    },
    { content, pubkey: TEST_IDENTITIES.alice.pubkey },
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, { relaySelf: RELAY_SELF });
});

test("the wake is not in the DM timeline and the DM header does not change", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  const title = page.getByTestId("chat-title");
  const titleBefore = await title.innerText();

  await emitWake(page, RELAY_SELF);
  // A message sent after the wake proves the stream delivered both in order.
  await emitPlain(page, "Hello from Alice after the wake");

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("Hello from Alice after the wake");
  await expect(timeline).not.toContainText("Wake from the relay");
  await expect(timeline).not.toContainText(WAKE_TEXT);
  await expect(title).toHaveText(titleBefore);
});

test("the wake adds no unread count to the DM row in the sidebar", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  await page.getByTestId("channel-general").click();

  await emitWake(page, RELAY_SELF);
  await emitPlain(page, "A real unread message");

  // Exactly one unread: the real message. The wake did not count.
  await expectUnreadBadgeCount(
    page.getByTestId("channel-unread-alice-tyler"),
    1,
  );
});

test("the wake is not an Activity row, while a normal message is", async ({
  page,
}) => {
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );
  const wakeId = "d1".repeat(32);
  const plainId = "d2".repeat(32);
  await page.evaluate(
    ({ channelId, wakeId, plainId, relay, alice, text }) => {
      const push = (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      const now = Math.floor(Date.now() / 1000);
      push?.({
        id: plainId,
        kind: 9,
        pubkey: alice,
        content: "A normal message that mentions you",
        created_at: now - 5,
        channel_id: channelId,
        channel_name: "general",
        tags: [["h", channelId]],
        category: "mention",
      });
      push?.({
        id: wakeId,
        kind: 9,
        pubkey: relay,
        content: text,
        created_at: now,
        channel_id: channelId,
        channel_name: "general",
        tags: [
          ["h", channelId],
          ["buzz:cofounder-wake", "mode_on"],
        ],
        category: "mention",
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      wakeId,
      plainId,
      relay: RELAY_SELF,
      alice: TEST_IDENTITIES.alice.pubkey,
      text: WAKE_TEXT,
    },
  );

  await page.getByTestId("sidebar-activity-button").click();
  await expect(page.getByTestId(`home-inbox-item-${plainId}`)).toBeVisible();
  await expect(page.getByTestId(`home-inbox-item-${wakeId}`)).toHaveCount(0);
  await expect(page.getByTestId("home-inbox-list")).not.toContainText(
    "Wake from the relay",
  );
});

test("a member who copies the tag is still seen: only the relay's own wake is hidden", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await waitForMockLiveSubscription(page, "alice-tyler");

  await emitWake(page, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("message-timeline")).toContainText(
    "Wake from the relay",
  );
});
