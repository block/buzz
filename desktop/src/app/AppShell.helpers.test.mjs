import assert from "node:assert/strict";
import test from "node:test";

import {
  markAllReadSources,
  shouldBounceForChannelNotification,
} from "./AppShell.helpers.ts";

test("shouldBounceForChannelNotification_allowsTopLevelChannelMessages", () => {
  assert.equal(shouldBounceForChannelNotification([["h", "channel"]]), true);
});

test("shouldBounceForChannelNotification_suppressesThreadReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ]),
    false,
  );
});

test("shouldBounceForChannelNotification_allowsBroadcastReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ]),
    true,
  );
});

test("markAllReadSources clears Inbox overrides and active thread activity", () => {
  const calls = [];

  markAllReadSources({
    activeChannelId: "active-channel",
    channelActivityItems: [
      { channelId: "another-channel", createdAt: 100 },
      { channelId: "active-channel", createdAt: 200 },
      { channelId: "active-channel", createdAt: 300 },
    ],
    unreadFeedItemIds: new Set(["first-inbox-item", "second-inbox-item"]),
    undoUnreadFeedItem: (itemId) => calls.push(`inbox:${itemId}`),
    markAllChannelReadMarkers: () => calls.push("channels"),
    markActiveChannelRead: (channelId, createdAt) =>
      calls.push(`active:${channelId}:${createdAt}`),
  });

  assert.deepEqual(calls, [
    "inbox:first-inbox-item",
    "inbox:second-inbox-item",
    "channels",
    "active:active-channel:300",
  ]);
});

test("markAllReadSources skips the active marker without projected activity", () => {
  const calls = [];

  markAllReadSources({
    activeChannelId: "active-channel",
    channelActivityItems: [],
    unreadFeedItemIds: new Set(),
    undoUnreadFeedItem: () => calls.push("inbox"),
    markAllChannelReadMarkers: () => calls.push("channels"),
    markActiveChannelRead: () => calls.push("active"),
  });

  assert.deepEqual(calls, ["channels"]);
});
