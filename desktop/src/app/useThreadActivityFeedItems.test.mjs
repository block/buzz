import assert from "node:assert/strict";
import test from "node:test";

import { buildThreadActivityFeedItems } from "./useThreadActivityFeedItems.ts";

const CHANNEL_ID = "channel-1";
const OTHER_CHANNEL_ID = "channel-2";

function threadActivityItem(overrides) {
  const rootId = overrides.rootId ?? "root-1";
  return {
    id: overrides.id ?? "reply-1",
    kind: overrides.kind ?? 9,
    pubkey: overrides.pubkey ?? "author",
    content: overrides.content ?? "reply",
    createdAt: overrides.createdAt ?? 1,
    channelId: overrides.channelId ?? CHANNEL_ID,
    channelName: overrides.channelName ?? "general",
    tags: overrides.tags ?? [
      ["h", overrides.channelId ?? CHANNEL_ID],
      ["e", rootId, "", "root"],
      ["e", overrides.parentId ?? "parent-1", "", "reply"],
    ],
  };
}

test("thread activity feed projection filters muted roots", () => {
  const items = buildThreadActivityFeedItems(
    [
      threadActivityItem({ id: "muted-reply", rootId: "muted-root" }),
      threadActivityItem({ id: "visible-reply", rootId: "visible-root" }),
    ],
    new Set(["muted-root"]),
    [{ id: CHANNEL_ID, name: "general", channelType: "stream" }],
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["visible-reply"],
  );
  assert.equal(items[0]?.category, "activity");
  assert.equal(items[0]?.channelType, "stream");
});

test("channel-presence fence: items from channels absent in community are filtered out", () => {
  // community A has channel-1; community B (relayed) has only channel-2.
  // Simulate A's stored items landing in B's projection call — they should
  // all be filtered out because channel-1 is not in B's channel set.
  const communityBChannels = [
    { id: OTHER_CHANNEL_ID, name: "other-channel", channelType: "stream" },
  ];

  const items = buildThreadActivityFeedItems(
    [
      threadActivityItem({ id: "reply-from-a", channelId: CHANNEL_ID }),
      threadActivityItem({
        id: "reply-from-b",
        channelId: OTHER_CHANNEL_ID,
      }),
    ],
    new Set(),
    communityBChannels,
  );

  // Only the item whose channelId exists in the current community passes.
  assert.deepEqual(
    items.map((item) => item.id),
    ["reply-from-b"],
  );
});

test("channel-presence fence: all items pass when all channels are present", () => {
  const items = buildThreadActivityFeedItems(
    [
      threadActivityItem({ id: "reply-a", channelId: CHANNEL_ID }),
      threadActivityItem({ id: "reply-b", channelId: OTHER_CHANNEL_ID }),
    ],
    new Set(),
    [
      { id: CHANNEL_ID, name: "general", channelType: "stream" },
      { id: OTHER_CHANNEL_ID, name: "other", channelType: "stream" },
    ],
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["reply-a", "reply-b"],
  );
});

test("channel-presence fence: empty items list returns empty feed", () => {
  const items = buildThreadActivityFeedItems([], new Set(), [
    { id: CHANNEL_ID, name: "general", channelType: "stream" },
  ]);

  assert.deepEqual(items, []);
});

test("channel-presence fence applies before mute filter — unknown channel + muted root still filtered", () => {
  // An item from a foreign channel with a muted root should be removed by
  // the channel-presence fence; the mute filter is irrelevant here.
  const items = buildThreadActivityFeedItems(
    [
      threadActivityItem({
        id: "foreign-muted",
        channelId: OTHER_CHANNEL_ID,
        rootId: "muted-root",
      }),
    ],
    new Set(["muted-root"]),
    [{ id: CHANNEL_ID, name: "general", channelType: "stream" }],
  );

  assert.deepEqual(items, []);
});

test("top-level DM activity is gated by the Activity experiment", () => {
  const dmItem = threadActivityItem({
    id: "agent-dm-reply",
    tags: [["h", CHANNEL_ID]],
  });
  const channels = [{ id: CHANNEL_ID, name: "Agent DM", channelType: "dm" }];

  assert.deepEqual(
    buildThreadActivityFeedItems([dmItem], new Set(), channels).map(
      (item) => item.id,
    ),
    [],
  );
  assert.deepEqual(
    buildThreadActivityFeedItems([dmItem], new Set(), channels, true).map(
      (item) => item.id,
    ),
    ["agent-dm-reply"],
  );
});
