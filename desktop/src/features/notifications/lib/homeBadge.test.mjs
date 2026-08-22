import assert from "node:assert/strict";
import test from "node:test";

import {
  buildHomeBadgeFeedItems,
  isMutedOutOfBadgeCount,
} from "./homeBadge.ts";

const ROOT = "a".repeat(64);
const PARENT = "b".repeat(64);

function feedWith(mentions) {
  return {
    feed: {
      mentions,
      needsAction: [],
      activity: [],
      agentActivity: [],
    },
  };
}

function replyItem(id, overrides = {}) {
  return {
    id,
    channelId: "chan-1",
    channelType: "channel",
    category: "mention",
    createdAt: 100,
    tags: [
      ["e", ROOT, "", "root"],
      ["e", PARENT, "", "reply"],
    ],
    ...overrides,
  };
}

test("a reply to your own message is kept out of the badge count", () => {
  // It reaches the mention feed only via its NIP-10 addressing `p` tag, which is
  // byte-identical to a typed @mention. The toast path already skips it, and
  // this count has no thread-mute input — so counting it would both disagree
  // with what the user was shown and let a muted thread drive the dock badge.
  const items = buildHomeBadgeFeedItems(
    feedWith([replyItem("reply-1", { replyToSelf: true })]),
    [],
    new Set(),
  );

  assert.deepEqual(items, []);
});

test("a typed mention that is also a reply still counts", () => {
  // Mentions are meant to pierce mutes; the backend marks these replyToSelf:false.
  const items = buildHomeBadgeFeedItems(
    feedWith([replyItem("mention-1", { replyToSelf: false })]),
    [],
    new Set(),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["mention-1"],
  );
});

test("an explicit mark-as-unread outranks the reply exclusion", () => {
  // The Inbox still renders these rows, so dropping one the user deliberately
  // marked unread left the row's dot showing while the numeral stayed at 0.
  const items = buildHomeBadgeFeedItems(
    feedWith([replyItem("reply-1", { replyToSelf: true })]),
    [],
    new Set(["reply-1"]),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["reply-1"],
  );
});

test("a top-level mention with no reply tags counts even if replyToSelf is set", () => {
  // replyToSelf only means "addressing tag" on something that is actually a
  // reply. Without the thread tags there is no addressing tag to discount.
  const items = buildHomeBadgeFeedItems(
    feedWith([
      {
        id: "top-1",
        channelId: "chan-1",
        channelType: "channel",
        category: "mention",
        createdAt: 100,
        tags: [["p", "c".repeat(64)]],
        replyToSelf: true,
      },
    ]),
    [],
    new Set(),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["top-1"],
  );
});

test("a mark-as-unread reply survives BOTH badge gates in a muted channel", () => {
  // The regression this guards: the two gates cancelled each other. The build
  // step admitted the item, then the counting loop's mute clause re-tested
  // `replyToSelf` and dropped it again — so the numeral stayed at zero while the
  // Inbox row showed its dot. Testing `buildHomeBadgeFeedItems` alone missed it,
  // which is why both halves are asserted together here.
  const muted = new Set(["chan-1"]);
  const item = replyItem("reply-1", { replyToSelf: true });

  const items = buildHomeBadgeFeedItems(
    feedWith([item]),
    [],
    new Set(["reply-1"]),
  );
  assert.deepEqual(
    items.map((entry) => entry.id),
    ["reply-1"],
  );
  assert.equal(isMutedOutOfBadgeCount(items[0], muted), false);
});

test("a muted channel's non-mention activity stays out of the count", () => {
  assert.equal(
    isMutedOutOfBadgeCount(
      { category: "activity", channelId: "chan-1" },
      new Set(["chan-1"]),
    ),
    true,
  );
});

test("a real mention pierces a channel mute", () => {
  assert.equal(
    isMutedOutOfBadgeCount(
      { category: "mention", channelId: "chan-1" },
      new Set(["chan-1"]),
    ),
    false,
  );
});

test("an answer inside a DM still counts toward the numeral", () => {
  // Every DM message p-tags both participants, so `replyToSelf` carries no
  // information there — the addressing tag *is* the addressing. Without the
  // carve-out, Alice's first DM counted but her reply to my answer did not.
  const items = buildHomeBadgeFeedItems(
    feedWith([replyItem("dm-reply", { replyToSelf: true })]),
    [],
    new Set(),
    new Set(["chan-1"]),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["dm-reply"],
  );
});

test("an answer in a non-DM channel is still excluded", () => {
  const items = buildHomeBadgeFeedItems(
    feedWith([replyItem("chan-reply", { replyToSelf: true })]),
    [],
    new Set(),
    new Set(["some-other-dm"]),
  );

  assert.deepEqual(items, []);
});
