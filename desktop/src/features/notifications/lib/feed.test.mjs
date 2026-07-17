import assert from "node:assert/strict";
import test from "node:test";

import { enrichFeedItemChannel, notificationTitle } from "./feed.ts";

const feedItem = (overrides = {}) => ({
  id: "event-id",
  kind: 46011,
  pubkey: "author",
  content: "Please review",
  createdAt: 1,
  channelId: "channel-id",
  channelName: "",
  channelType: undefined,
  tags: [["h", "channel-id"]],
  category: "needs_action",
  ...overrides,
});

const channels = [
  { id: "channel-id", name: "ship-room", channelType: "stream" },
];

test("enriches a feed notification with its loaded channel name", () => {
  const item = enrichFeedItemChannel(feedItem(), channels);

  assert.equal(item.channelName, "ship-room");
  assert.equal(item.channelType, "stream");
  assert.equal(notificationTitle(item), "Needs Action in #ship-room");
});

test("preserves feed-provided channel metadata", () => {
  const original = feedItem({
    channelName: "backend-name",
    channelType: "forum",
  });
  const item = enrichFeedItemChannel(original, channels);

  assert.equal(item, original);
  assert.equal(notificationTitle(item), "Needs Action in #backend-name");
});

test("falls back safely when the channel list has not loaded the channel", () => {
  const original = feedItem();
  const item = enrichFeedItemChannel(original, []);

  assert.equal(item, original);
  assert.equal(notificationTitle(item), "Needs Action");
});

test("does not replace direct-message notification titles", () => {
  const original = feedItem({ channelType: "dm" });
  const item = enrichFeedItemChannel(original, [
    { id: "channel-id", name: "not-a-title", channelType: "dm" },
  ]);

  assert.equal(notificationTitle(item, "Taylor"), "Taylor");
});
