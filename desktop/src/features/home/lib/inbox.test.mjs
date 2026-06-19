import assert from "node:assert/strict";
import test from "node:test";

import { buildInboxItems, getInboxTypeLabel } from "./inbox.ts";

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
