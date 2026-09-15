import assert from "node:assert/strict";
import test from "node:test";

import {
  getGroupedChannelReadTimestamp,
  getGroupedInboxItemIds,
  hasRemainingChannelUnreadOverride,
  hasGroupedUnreadOverride,
  projectInboxDoneSet,
  resolveInboxItemReadAt,
} from "./useHomeInboxReadState.ts";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

function feedItem(overrides) {
  return {
    id: overrides.id,
    kind: 9,
    pubkey: "author",
    content: "hello",
    createdAt: overrides.createdAt,
    channelId: Object.hasOwn(overrides, "channelId")
      ? overrides.channelId
      : CHANNEL_ID,
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
    latestActivityAt: Math.max(...groupItems.map((entry) => entry.createdAt)),
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

test("remaining channel unread override ignores the row being cleared", () => {
  const firstReply = feedItem({
    id: "first-reply",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "first-root", "", "root"],
      ["e", "first-parent", "", "reply"],
    ],
  });
  const secondReply = feedItem({
    id: "second-reply",
    createdAt: 300,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "second-root", "", "root"],
      ["e", "second-parent", "", "reply"],
    ],
  });
  const items = [inboxItem([firstReply]), inboxItem([secondReply])];

  assert.equal(
    hasRemainingChannelUnreadOverride(
      items,
      new Set(["first-reply", "second-reply"]),
      CHANNEL_ID,
      new Set(["first-reply"]),
    ),
    true,
  );
  assert.equal(
    hasRemainingChannelUnreadOverride(
      items,
      new Set(["first-reply"]),
      CHANNEL_ID,
      new Set(["first-reply"]),
    ),
    false,
  );
});

test("thread inbox row without a marker ignores local done fallback", () => {
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
    resolveInboxItemReadAt(inboxItem([replyItem]), {
      getChannelReadAt: () => 100,
      getThreadReadAt: () => null,
      getMessageReadAt: () => null,
    }),
    null,
  );
});

test("thread inbox row read state includes per-message marker", () => {
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
    resolveInboxItemReadAt(inboxItem([replyItem]), {
      getChannelReadAt: () => 100,
      getThreadReadAt: () => null,
      getMessageReadAt: (messageId) =>
        messageId === "reply-event" ? 200 : null,
    }),
    200,
  );
});

test("thread inbox row read state follows the per-message marker", () => {
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
    resolveInboxItemReadAt(inboxItem([replyItem]), {
      getChannelReadAt: () => 100,
      getThreadReadAt: () => 250,
      getMessageReadAt: () => 200,
    }),
    200,
  );
});

test("parent-only replies use their parent as the thread read context", () => {
  const replyItem = feedItem({
    id: "reply-event",
    createdAt: 200,
    tags: [
      ["h", CHANNEL_ID],
      ["e", "parent-event", "", "reply"],
    ],
  });
  let resolvedRootId = null;

  assert.equal(
    resolveInboxItemReadAt(inboxItem([replyItem]), {
      getChannelReadAt: () => 100,
      getThreadReadAt: (rootId) => {
        resolvedRootId = rootId;
        return 150;
      },
    }),
    150,
  );
  assert.equal(resolvedRootId, "parent-event");
});

test("unknown channel markers before NIP-RS hydrate are done, not unread", () => {
  const channelRow = inboxItem([
    feedItem({
      id: "channel-event",
      createdAt: 200,
    }),
  ]);
  const threadRow = inboxItem([
    feedItem({
      id: "reply-event",
      createdAt: 200,
      tags: [
        ["h", CHANNEL_ID],
        ["e", "root-event", "", "root"],
        ["e", "parent-event", "", "reply"],
      ],
    }),
  ]);

  const done = projectInboxDoneSet([channelRow, threadRow], {
    getChannelReadAt: () => null,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: false,
    localDoneSet: new Set(),
    localUnreadSet: new Set(),
  });

  assert.equal(done.has("channel-event"), true);
  assert.equal(done.has("reply-event"), true);
});

test("unknown channel markers after NIP-RS hydrate stay unread", () => {
  const channelRow = inboxItem([
    feedItem({
      id: "channel-event",
      createdAt: 200,
    }),
  ]);

  const done = projectInboxDoneSet([channelRow], {
    getChannelReadAt: () => null,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: true,
    localDoneSet: new Set(),
    localUnreadSet: new Set(),
  });

  assert.equal(done.has("channel-event"), false);
});

test("known unread markers stay unread during NIP-RS hydrate", () => {
  const channelRow = inboxItem([
    feedItem({
      id: "channel-event",
      createdAt: 200,
    }),
  ]);

  const done = projectInboxDoneSet([channelRow], {
    getChannelReadAt: () => 100,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: false,
    localDoneSet: new Set(),
    localUnreadSet: new Set(),
  });

  assert.equal(done.has("channel-event"), false);
});

test("local unread override still wins during NIP-RS hydrate", () => {
  const channelRow = inboxItem([
    feedItem({
      id: "channel-event",
      createdAt: 200,
    }),
  ]);

  const done = projectInboxDoneSet([channelRow], {
    getChannelReadAt: () => null,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: false,
    localDoneSet: new Set(),
    localUnreadSet: new Set(["channel-event"]),
  });

  assert.equal(done.has("channel-event"), false);
});

test("non-channel rows still use the local done-set during NIP-RS hydrate", () => {
  const reminderRow = inboxItem([
    feedItem({
      id: "reminder-event",
      channelId: null,
      createdAt: 200,
      tags: [],
    }),
  ]);

  const unread = projectInboxDoneSet([reminderRow], {
    getChannelReadAt: () => null,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: false,
    localDoneSet: new Set(),
    localUnreadSet: new Set(),
  });
  const done = projectInboxDoneSet([reminderRow], {
    getChannelReadAt: () => null,
    getMessageReadAt: () => null,
    getThreadReadAt: () => null,
    isReadStateReady: false,
    localDoneSet: new Set(["reminder-event"]),
    localUnreadSet: new Set(),
  });

  assert.equal(unread.has("reminder-event"), false);
  assert.equal(done.has("reminder-event"), true);
});
