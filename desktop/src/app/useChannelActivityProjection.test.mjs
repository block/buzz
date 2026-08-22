import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeUnreadThreadChannelIds,
  resolveChannelActivityFeedItemReadAt,
} from "./useChannelActivityProjection.ts";

test("sidebar thread channels include authoritative unread IDs beyond the preview cap", () => {
  const channelIds = mergeUnreadThreadChannelIds(
    new Set(["preview-channel"]),
    new Map([
      ["authoritative-channel", new Set(["older-reply"])],
      ["read-channel", new Set()],
    ]),
  );

  assert.deepEqual([...channelIds].sort(), [
    "authoritative-channel",
    "preview-channel",
  ]);
});

test("channel activity read state folds the item's own message and channel markers", () => {
  const markers = new Map([
    ["msg:reply-general", 100],
    ["general", 200],
    ["random", 500],
  ]);

  assert.equal(
    resolveChannelActivityFeedItemReadAt(
      { id: "reply-general", channelId: "general" },
      (contextId) => markers.get(contextId) ?? null,
    ),
    200,
  );
});

test("channel activity read state honors a channel marker without a message marker", () => {
  assert.equal(
    resolveChannelActivityFeedItemReadAt(
      { id: "reply-general", channelId: "general" },
      (contextId) => (contextId === "general" ? 300 : null),
    ),
    300,
  );
});

test("channel activity read state ignores the passive timeline marker", () => {
  const markers = new Map([
    ["general", 100],
    ["channel-timeline:general", 500],
  ]);

  assert.equal(
    resolveChannelActivityFeedItemReadAt(
      { id: "reply-general", channelId: "general" },
      (contextId) => markers.get(contextId) ?? null,
    ),
    100,
  );
});
