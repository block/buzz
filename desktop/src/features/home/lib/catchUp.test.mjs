import assert from "node:assert/strict";
import test from "node:test";

import { buildCatchUpDigest } from "./catchUp.ts";

const NOW = 1_700_000_000;

function makeFeedItem(overrides = {}) {
  const {
    id = "msg-1",
    channelId = "channel-1",
    channelName = "general",
    content = "Engineering shipped the desktop build.",
    createdAt = NOW - 600,
    pubkey = "a".repeat(64),
    category = "activity",
    tags = [],
    kind = 9,
  } = overrides;
  return {
    id,
    kind,
    pubkey,
    content,
    createdAt,
    channelId,
    channelName,
    tags,
    category,
  };
}

function makeInboxItem(overrides = {}) {
  const {
    conversationId = "conv-1",
    categories = ["activity"],
    groupItems,
    item = makeFeedItem(),
    latestActivityAt = item.createdAt,
    preview = item.content,
  } = overrides;
  return {
    avatarUrl: null,
    conversationId,
    id: item.id,
    item,
    categories,
    categoryLabel: "Activity",
    channelLabel: item.channelName,
    fullTimestampLabel: "",
    groupItems: groupItems ?? [item],
    isActionRequired: categories.includes("needs_action"),
    latestActivityAt,
    mentionNames: [],
    preview,
    senderLabel: "Alice",
    subject: preview,
    timestampLabel: "",
    unreadCount: 1,
  };
}

const noReadMarker = () => null;

test("unread non-attention items become channel-grouped lines", () => {
  const digest = buildCatchUpDigest({
    items: [makeInboxItem()],
    doneSet: new Set(),
    getItemReadAt: noReadMarker,
  });
  assert.equal(digest.groups.length, 1);
  assert.equal(digest.groups[0].channelId, "channel-1");
  assert.equal(digest.groups[0].lines.length, 1);
  assert.equal(
    digest.groups[0].lines[0].summary,
    "Engineering shipped the desktop build.",
  );
  assert.equal(digest.needsYouCount, 0);
});

test("read conversations and attention asks are excluded from lines", () => {
  const readItem = makeInboxItem();
  const mention = makeInboxItem({
    conversationId: "conv-m",
    categories: ["mention"],
    item: makeFeedItem({ id: "msg-m", content: "Can you review the plan?" }),
  });
  const digest = buildCatchUpDigest({
    items: [readItem, mention],
    doneSet: new Set([readItem.id]),
    getItemReadAt: noReadMarker,
  });
  assert.equal(digest.groups.length, 0);
  assert.equal(digest.needsYouCount, 1);
  assert.equal(digest.totalLineCount, 0);
});

test("lines read oldest first, groups order by most recent activity", () => {
  const quietChannel = makeInboxItem({
    conversationId: "conv-quiet",
    item: makeFeedItem({
      id: "msg-quiet",
      channelId: "channel-quiet",
      channelName: "quiet",
      createdAt: NOW - 9_000,
    }),
  });
  const older = makeFeedItem({ id: "msg-old", createdAt: NOW - 5_000 });
  const newer = makeFeedItem({ id: "msg-new", createdAt: NOW - 100 });
  const busyChannel = makeInboxItem({
    conversationId: "conv-busy",
    item: newer,
    groupItems: [newer, older],
  });
  const digest = buildCatchUpDigest({
    items: [quietChannel, busyChannel],
    doneSet: new Set(),
    getItemReadAt: noReadMarker,
  });
  assert.deepEqual(
    digest.groups.map((group) => group.channelId),
    ["channel-1", "channel-quiet"],
  );
  assert.deepEqual(
    digest.groups[0].lines.map((line) => line.id),
    ["msg-old", "msg-new"],
  );
});

test("a read boundary hides messages at or before the marker", () => {
  const older = makeFeedItem({ id: "msg-old", createdAt: NOW - 5_000 });
  const newer = makeFeedItem({ id: "msg-new", createdAt: NOW - 100 });
  const item = makeInboxItem({ item: newer, groupItems: [newer, older] });
  const digest = buildCatchUpDigest({
    items: [item],
    doneSet: new Set(),
    getItemReadAt: () => NOW - 1_000,
  });
  assert.deepEqual(
    digest.groups[0].lines.map((line) => line.id),
    ["msg-new"],
  );
});

test("channels cap at 10 lines with an honest remainder", () => {
  const messages = Array.from({ length: 13 }, (_, index) =>
    makeFeedItem({ id: `msg-${index}`, createdAt: NOW - 10_000 + index * 60 }),
  );
  const item = makeInboxItem({
    item: messages[messages.length - 1],
    groupItems: messages,
  });
  const digest = buildCatchUpDigest({
    items: [item],
    doneSet: new Set(),
    getItemReadAt: noReadMarker,
  });
  assert.equal(digest.groups[0].lines.length, 10);
  assert.equal(digest.groups[0].moreCount, 3);
  assert.equal(digest.groups[0].lines[0].id, "msg-0");
  assert.equal(digest.totalLineCount, 13);
});

test("items without a channel are skipped (no deep-link target)", () => {
  const digest = buildCatchUpDigest({
    items: [makeInboxItem({ item: makeFeedItem({ channelId: null }) })],
    doneSet: new Set(),
    getItemReadAt: noReadMarker,
  });
  assert.equal(digest.groups.length, 0);
});
