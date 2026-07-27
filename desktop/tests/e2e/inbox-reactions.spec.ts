import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import type { RelayEvent } from "../../src/shared/api/types";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const SHOTS = "test-results/inbox-reactions";

type MockWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    pubkey?: string;
    mentionPubkeys?: string[];
    /** 64-hex id — required for the message to be a valid reaction target. */
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

// Regression test: a reaction added from the Inbox detail pane must survive
// the post-toggle refetch. Inbox items are typically thread replies, which the
// server-assembled channel window does NOT carry reactions for — the Inbox
// must hydrate them by `#e` reference or the optimistic pill vanishes.
test("inbox reaction on a thread-reply mention persists after refetch", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(() => {
    const win = window as MockWindow;
    return (
      typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function"
    );
  });

  // Seed a thread: alice's top-level message, then alice's reply mentioning
  // tyler (the current user). The reply is the inbox item — the shape Wes hit.
  // A second, unrelated mention is seeded so the test can genuinely switch
  // selection away and back.
  const { replyEvent, otherEvent } = await page.evaluate(
    ({ channelId, currentPubkey, senderPubkey }) => {
      const win = window as MockWindow;
      const emitMessage = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const pushFeedItem = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emitMessage || !pushFeedItem) {
        throw new Error("Mock bridge helpers are not installed.");
      }

      const root = emitMessage({
        channelName: "general",
        content: "Thread root about the launch.",
        pubkey: senderPubkey,
        id: "a1".repeat(32),
      });
      const reply = emitMessage({
        channelName: "general",
        content: "Reply mentioning you — please react to this.",
        parentEventId: root.id,
        pubkey: senderPubkey,
        mentionPubkeys: [currentPubkey],
        id: "b2".repeat(32),
      });

      pushFeedItem({
        id: reply.id,
        kind: reply.kind,
        pubkey: reply.pubkey,
        content: reply.content,
        created_at: reply.created_at,
        channel_id: channelId,
        channel_name: "general",
        tags: reply.tags,
        category: "mention",
      });

      const other = emitMessage({
        channelName: "general",
        content: "Unrelated mention for selection switching.",
        pubkey: senderPubkey,
        mentionPubkeys: [currentPubkey],
        id: "c3".repeat(32),
      });
      pushFeedItem({
        id: other.id,
        kind: other.kind,
        pubkey: other.pubkey,
        content: other.content,
        created_at: other.created_at,
        channel_id: channelId,
        channel_name: "general",
        tags: other.tags,
        category: "mention",
      });
      return { replyEvent: reply, otherEvent: other };
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );

  // Open the inbox item and react via the hover action bar's quick reaction.
  const item = page.getByTestId(`home-inbox-item-${replyEvent.id}`);
  await item.click();
  const detail = page.getByTestId("home-inbox-detail");
  await expect(detail).toContainText("please react to this");

  const selectedMessage = page.getByTestId("home-inbox-selected-message");

  // Deliver a live reaction with the real add_reaction wire shape: `e` target,
  // no `h` channel tag. The Inbox must render it without waiting for another
  // message or a context refetch.
  await page.evaluate(async (eventId) => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("Mock Tauri invoke bridge is unavailable.");
    await invoke("add_reaction", { eventId, emoji: "❤️" });
  }, replyEvent.id);
  await expect(selectedMessage.getByLabel("Toggle ❤️ reaction")).toBeVisible();

  await selectedMessage.hover();
  const actionBar = page.getByTestId(`message-action-bar-${replyEvent.id}`);
  const [actionBarBox, selectedMessageBox] = await Promise.all([
    actionBar.boundingBox(),
    selectedMessage.boundingBox(),
  ]);
  expect(actionBarBox).not.toBeNull();
  expect(selectedMessageBox).not.toBeNull();
  if (!actionBarBox || !selectedMessageBox) {
    throw new Error("Inbox message action bar bounds were unavailable.");
  }
  expect(actionBarBox.y).toBeGreaterThanOrEqual(selectedMessageBox.y);

  await selectedMessage
    .getByRole("button", { name: "React with :+1:" })
    .click();

  // The pill must appear AND persist: the post-toggle refetch replaces the
  // optimistic state with fetched reaction events. Give the refetch time to
  // land before asserting, then assert the pill is still there.
  const reactions = selectedMessage.getByTestId("message-reactions");
  await expect(reactions).toContainText("👍");
  await page.waitForTimeout(1_500);
  await expect(reactions).toContainText("👍");
  await page.screenshot({ path: `${SHOTS}/01-pill-after-refetch.png` });

  // Re-select the item (drops all optimistic state) — the reaction must
  // come back purely from the fetched relay data. Select a genuinely
  // different item first so the selection actually changes.
  const otherItem = page.getByTestId(`home-inbox-item-${otherEvent.id}`);
  await otherItem.click();
  await expect(detail).toContainText("Unrelated mention");
  await item.click();
  await expect(detail).toContainText("please react to this");
  await expect(
    page
      .getByTestId("home-inbox-selected-message")
      .getByTestId("message-reactions"),
  ).toContainText("👍");
  await page.screenshot({ path: `${SHOTS}/02-pill-after-reselect.png` });
});
