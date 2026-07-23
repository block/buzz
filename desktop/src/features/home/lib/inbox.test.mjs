import assert from "node:assert/strict";
import test from "node:test";

import {
  buildInboxItems,
  getInboxConversationId,
  getInboxTypeLabel,
} from "./inbox.ts";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

const channels = [
  {
    id: CHANNEL_ID,
    name: "buzz-bugs",
    channelType: "stream",
  },
];

function feedWith(overrides) {
  return {
    feed: {
      mentions: overrides.mentions ?? [],
      needsAction: overrides.needsAction ?? [],
      activity: overrides.activity ?? [],
      agentActivity: overrides.agentActivity ?? [],
    },
    meta: {
      since: 0,
      total: 0,
      generatedAt: 0,
    },
  };
}

function item(overrides) {
  return {
    id: overrides.id ?? "event-1",
    kind: overrides.kind ?? 9,
    pubkey: overrides.pubkey ?? "author",
    content: overrides.content ?? "hello",
    createdAt: overrides.createdAt ?? 1,
    channelId: overrides.channelId ?? CHANNEL_ID,
    channelName: overrides.channelName ?? "",
    tags: overrides.tags ?? [["h", CHANNEL_ID]],
    category: overrides.category ?? "mention",
  };
}

test("mention rows use the channel list when feed channelName is blank", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [item({ category: "mention" })],
    }),
  });

  assert.deepEqual(getInboxTypeLabel(inboxItem), {
    text: "Mentioned in",
    channelLabel: "buzz-bugs",
  });
});

test("thread activity rows use the channel list when feed channelName is blank", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          category: "activity",
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "parent-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  assert.deepEqual(getInboxTypeLabel(inboxItem), {
    text: "Thread in",
    channelLabel: "buzz-bugs",
  });
});

test("thread groups are represented by the latest reply rather than the root", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: "root-event",
          category: "activity",
          content: "Original thread starter",
          createdAt: 1,
        }),
        item({
          id: "reply-event",
          category: "activity",
          content: "New reply in the thread",
          createdAt: 2,
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "parent-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  assert.equal(inboxItem.id, "reply-event");
  assert.equal(inboxItem.preview, "New reply in the thread");
  assert.deepEqual(
    inboxItem.groupItems.map((groupItem) => groupItem.id),
    ["root-event", "reply-event"],
  );
  assert.deepEqual(getInboxTypeLabel(inboxItem), {
    text: "Thread in",
    channelLabel: "buzz-bugs",
  });
});

test("thread groups use the latest row label even when the root was a mention", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [
        item({
          id: "root-event",
          category: "mention",
          content: "Original mention",
          createdAt: 1,
        }),
      ],
      activity: [
        item({
          id: "reply-event",
          category: "activity",
          content: "New reply in the thread",
          createdAt: 2,
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "parent-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  assert.equal(inboxItem.id, "reply-event");
  assert.deepEqual(
    inboxItem.groupItems.map((groupItem) => groupItem.id),
    ["root-event", "reply-event"],
  );
  assert.deepEqual(getInboxTypeLabel(inboxItem), {
    text: "Thread in",
    channelLabel: "buzz-bugs",
  });
});

// ── conversationId stability tests ──────────────────────────────────────────

test("conversationId is stable when a live reply advances the representative", () => {
  // Simulate initial feed: thread root is the latest item.
  const before = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: "root-event",
          category: "activity",
          createdAt: 1,
          tags: [["h", CHANNEL_ID]],
        }),
      ],
    }),
  });

  assert.equal(before.length, 1);
  const conversationIdBefore = before[0].conversationId;

  // Simulate live reply arriving — root-event is now no longer the latest.
  const after = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: "root-event",
          category: "activity",
          createdAt: 1,
          tags: [["h", CHANNEL_ID]],
        }),
        item({
          id: "reply-event",
          category: "activity",
          createdAt: 2,
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "root-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  assert.equal(after.length, 1);
  // The conversation is still the same group — conversationId must be stable.
  assert.equal(after[0].conversationId, conversationIdBefore);
  // Representative is now the reply (latest by createdAt).
  assert.equal(after[0].id, "reply-event");
});

test("conversationId equals the thread root event id when a root tag is present", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: "reply-event",
          category: "activity",
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "parent-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  assert.equal(inboxItem.conversationId, "root-event");
});

test("conversationId falls back to event id for a top-level item with no thread tags", () => {
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      mentions: [
        item({
          id: "top-level-event",
          category: "mention",
          tags: [["h", CHANNEL_ID]],
        }),
      ],
    }),
  });

  assert.equal(inboxItem.conversationId, "top-level-event");
  assert.equal(inboxItem.id, "top-level-event");
});

test("getInboxConversationId uses root tag when present", () => {
  const tags = [
    ["h", CHANNEL_ID],
    ["e", "root-event", "", "root"],
    ["e", "parent-event", "", "reply"],
  ];

  assert.equal(getInboxConversationId(tags, "reply-event"), "root-event");
});

test("getInboxConversationId falls back to eventId when no root tag", () => {
  const tags = [["h", CHANNEL_ID]];

  assert.equal(
    getInboxConversationId(tags, "top-level-event"),
    "top-level-event",
  );
});

test("old event still resolves to its conversation row via groupItems", () => {
  // Demonstrates that findItemByEventId searching groupItems works: the old
  // root event id is still present in groupItems even when a newer reply
  // becomes the representative.
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: "root-event",
          category: "activity",
          createdAt: 1,
          tags: [["h", CHANNEL_ID]],
        }),
        item({
          id: "reply-event",
          category: "activity",
          createdAt: 2,
          tags: [
            ["h", CHANNEL_ID],
            ["e", "root-event", "", "root"],
            ["e", "root-event", "", "reply"],
          ],
        }),
      ],
    }),
  });

  // The representative is the reply, but the root is still in groupItems.
  assert.equal(inboxItem.id, "reply-event");
  assert.ok(
    inboxItem.groupItems.some((groupItem) => groupItem.id === "root-event"),
    "root-event should be in groupItems",
  );
});

// ── nested-anchor retention: feed advance must not lose the anchor ───────────

test("nested-anchor: old selected event stays resolvable by conversationId after representative advances", () => {
  // Scenario: user selected "reply-1" (a non-root reply) as the anchor.
  // A new deeper reply arrives and becomes the new representative.
  // The feed now contains only "reply-2" (the new representative) plus the
  // root. "reply-1" has been displaced from the live feed window.
  //
  // Properties that must hold after the feed advance:
  //   1. The conversationId derived from the LATCHED anchor's tags matches
  //      the conversationId of the surviving InboxItem (so row stays selected).
  //   2. The surviving InboxItem is resolvable by that conversationId.
  //   3. The anchor event id ("reply-1") is NOT in the new groupItems —
  //      confirming the eviction we're testing against.

  const ROOT_ID = "root-event";
  const ANCHOR_EVENT_ID = "reply-1"; // what the user clicked
  const LATEST_EVENT_ID = "reply-2"; // new representative after feed advance

  // Build inboxItems from the feed AFTER the advance (reply-1 is gone).
  const [inboxItem] = buildInboxItems({
    channels,
    feed: feedWith({
      activity: [
        item({
          id: ROOT_ID,
          category: "activity",
          createdAt: 1,
          tags: [["h", CHANNEL_ID]],
        }),
        item({
          id: LATEST_EVENT_ID,
          category: "activity",
          createdAt: 3,
          tags: [
            ["h", CHANNEL_ID],
            ["e", ROOT_ID, "", "root"],
            ["e", ANCHOR_EVENT_ID, "", "reply"],
          ],
        }),
        // reply-1 is intentionally absent — it has been evicted.
      ],
    }),
  });

  // Property 3: anchor event is NOT in the new groupItems.
  assert.ok(
    !inboxItem.groupItems.some((gi) => gi.id === ANCHOR_EVENT_ID),
    "evicted anchor must not be in groupItems after feed advance",
  );

  // Property 1: conversationId derived from the latched anchor's tags
  // (what HomeView computes as latchedConversationId) equals the surviving
  // InboxItem's conversationId.
  const anchorTags = [
    ["h", CHANNEL_ID],
    ["e", ROOT_ID, "", "root"],
    ["e", "some-parent", "", "reply"],
  ];
  const latchedConversationId = getInboxConversationId(
    anchorTags,
    ANCHOR_EVENT_ID,
  );
  assert.equal(
    latchedConversationId,
    inboxItem.conversationId,
    "latchedConversationId must match the surviving InboxItem's conversationId",
  );

  // Property 2: the surviving row is resolvable by that conversationId.
  assert.equal(inboxItem.conversationId, ROOT_ID);
  assert.equal(latchedConversationId, ROOT_ID);

  // The new representative is the latest reply.
  assert.equal(inboxItem.id, LATEST_EVENT_ID);
});
