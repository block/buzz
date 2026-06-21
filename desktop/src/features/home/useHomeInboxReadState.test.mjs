import assert from "node:assert/strict";
import test from "node:test";

import {
  getGroupedChannelReadTimestamp,
  getGroupedInboxItemIds,
  hasGroupedUnreadOverride,
} from "./useHomeInboxReadState.ts";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

function feedItem(overrides) {
  return {
    id: overrides.id,
    kind: 9,
    pubkey: "author",
    content: "hello",
    createdAt: overrides.createdAt,
    channelId: overrides.channelId ?? CHANNEL_ID,
    channelName: "buzz-bugs",
    tags: overrides.tags ?? [["h", CHANNEL_ID]],
    category: overrides.category ?? "activity",
  };
}

function inboxItem(groupItems, item = groupItems.at(-1)) {
  return {
    id: item.id,
    item,
    groupItems,
  };
}

test("grouped channel read timestamp uses the root row, not the latest thread reply", () => {
  const rootItem = feedItem({
    id: "root-event",
    category: "mention",
    createdAt: 100,
  });
  const replyItem = feedItem({
    id: "reply-event",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "root-event", "", "root"],
      ["e", "parent-event", "", "reply"],
    ],
  });

  assert.deepEqual(
    getGroupedChannelReadTimestamp(inboxItem([rootItem, replyItem])),
    {
      channelId: CHANNEL_ID,
      timestamp: 100,
    },
  );
});

test("grouped channel read timestamp ignores thread-only groups", () => {
  const replyItem = feedItem({
    id: "reply-event",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "root-event", "", "root"],
      ["e", "parent-event", "", "reply"],
    ],
  });

  assert.equal(getGroupedChannelReadTimestamp(inboxItem([replyItem])), null);
});

test("grouped inbox item ids include every item represented by the row", () => {
  const rootItem = feedItem({
    id: "root-event",
    category: "mention",
    createdAt: 100,
  });
  const replyItem = feedItem({
    id: "reply-event",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "root-event", "", "root"],
      ["e", "parent-event", "", "reply"],
    ],
  });

  assert.deepEqual(getGroupedInboxItemIds(inboxItem([rootItem, replyItem])), [
    "reply-event",
    "root-event",
  ]);
});

test("grouped unread override matches any item represented by the row", () => {
  const rootItem = feedItem({
    id: "root-event",
    category: "mention",
    createdAt: 100,
  });
  const replyItem = feedItem({
    id: "reply-event",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "root-event", "", "root"],
      ["e", "parent-event", "", "reply"],
    ],
  });

  assert.equal(
    hasGroupedUnreadOverride(
      inboxItem([rootItem, replyItem]),
      new Set(["root-event"]),
    ),
    true,
  );
  assert.equal(
    hasGroupedUnreadOverride(
      inboxItem([rootItem, replyItem]),
      new Set(["other-event"]),
    ),
    false,
  );
});
