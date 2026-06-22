import assert from "node:assert/strict";
import test from "node:test";

import {
  buildHomeBadgeFeedItems,
  shouldCountTowardHomeBadgeSubtotal,
} from "./lib/homeBadge.ts";

const ROOT_TAGS = [
  ["h", "stream-channel"],
  ["e", "root-event", "", "root"],
  ["e", "parent-event", "", "reply"],
];

const feedItem = (id, category = "activity") => ({
  id,
  kind: 9,
  pubkey: "author",
  content: id,
  createdAt: 1,
  channelId: null,
  channelName: "",
  tags: [],
  category,
});

const homeFeed = (feed) => ({
  feed: {
    mentions: [],
    needsAction: [],
    activity: [],
    agentActivity: [],
    ...feed,
  },
  meta: { since: 0, total: 0, generatedAt: 0 },
});

test("home badge items include locally unread activity and agent rows", () => {
  const items = buildHomeBadgeFeedItems(
    homeFeed({
      mentions: [feedItem("mention", "mention")],
      needsAction: [feedItem("needs-action", "needs_action")],
      activity: [
        feedItem("locally-unread-activity"),
        feedItem("read-activity"),
      ],
      agentActivity: [
        feedItem("locally-unread-agent", "agent_activity"),
        feedItem("read-agent", "agent_activity"),
      ],
    }),
    [feedItem("thread-activity")],
    new Set(["locally-unread-activity", "locally-unread-agent"]),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    [
      "mention",
      "needs-action",
      "thread-activity",
      "locally-unread-activity",
      "locally-unread-agent",
    ],
  );
});

test("home badge subtotal excludes channel-counted high-priority items", () => {
  const highPriorityChannelIds = new Set(["dm-channel", "stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
    ),
    false,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "dm-channel", channelType: "dm", tags: ROOT_TAGS },
      highPriorityChannelIds,
    ),
    false,
  );
});

test("home badge subtotal still counts non-DM thread-only rows", () => {
  const highPriorityChannelIds = new Set(["stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: ROOT_TAGS },
      highPriorityChannelIds,
    ),
    true,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "main-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
    ),
    true,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: null, channelType: undefined, tags: [] },
      highPriorityChannelIds,
    ),
    true,
  );
});

test("home badge subtotal counts locally unread rows before channel exclusion", () => {
  const highPriorityChannelIds = new Set(["stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
      true,
    ),
    true,
  );
});
