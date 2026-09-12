import assert from "node:assert/strict";
import test from "node:test";

import {
  buildEventNotificationTarget,
  buildFeedItemNotificationTarget,
} from "./target.ts";

test("DM notification targets open replies in the channel timeline", () => {
  const target = buildEventNotificationTarget(
    {
      content: "hello",
      created_at: 123,
      id: "event-id",
      kind: 9,
      pubkey: "sender",
      tags: [
        ["h", "channel-id"],
        ["e", "root-id", "", "root"],
        ["e", "parent-id", "", "reply"],
      ],
    },
    { id: "channel-id", name: "ship-room" },
  );

  assert.deepEqual(target, {
    channelId: "channel-id",
    channelName: "ship-room",
    content: "hello",
    createdAt: 123,
    eventId: "event-id",
    kind: 9,
    pubkey: "sender",
    openInThread: false,
    threadRootId: null,
  });
});

test("thread-reply notification targets open the containing branch", () => {
  const target = buildEventNotificationTarget(
    {
      content: "hello",
      created_at: 123,
      id: "event-id",
      kind: 9,
      pubkey: "sender",
      tags: [
        ["h", "channel-id"],
        ["e", "root-id", "", "root"],
        ["e", "parent-id", "", "reply"],
      ],
    },
    { id: "channel-id", name: "ship-room" },
    { openInThread: true },
  );

  assert.equal(target.openInThread, true);
  assert.equal(target.threadRootId, "root-id");
});

test("null channel name and top-level events produce null fields", () => {
  const target = buildEventNotificationTarget(
    {
      content: "hello",
      created_at: 123,
      id: "event-id",
      kind: 9,
      pubkey: "sender",
      tags: [["h", "channel-id"]],
    },
    { id: "channel-id", name: "  " },
  );

  assert.equal(target.channelName, null);
  assert.equal(target.threadRootId, null);
});

test("builds a complete click-through target from a feed item", () => {
  const target = buildFeedItemNotificationTarget({
    id: "feed-event",
    kind: 9,
    pubkey: "sender",
    content: "ping",
    createdAt: 456,
    channelId: "channel-id",
    channelName: "ship-room",
    tags: [
      ["e", "root-id", "", "root"],
      ["e", "parent-id", "", "reply"],
    ],
    category: "mention",
  });

  assert.deepEqual(target, {
    channelId: "channel-id",
    channelName: "ship-room",
    content: "ping",
    createdAt: 456,
    eventId: "feed-event",
    kind: 9,
    pubkey: "sender",
    openInThread: true,
    threadRootId: "root-id",
  });
});
