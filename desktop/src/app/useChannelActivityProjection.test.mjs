import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveChannelActivityFeedItemReadAt,
  resolveThreadActivityItemReadAt,
} from "./useChannelActivityProjection.ts";

/** A thread reply tagged with its root, in NIP-10 shape. */
function threadReply(id, channelId, rootId) {
  return {
    id,
    channelId,
    tags: [
      ["e", rootId, "", "root"],
      ["e", rootId, "", "reply"],
    ],
  };
}

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

test("reading the thread clears its replies even with no message marker", () => {
  // The regression this guards: only the thread marker moved, and the channel
  // and per-message markers never did. Before folding the thread frontier in,
  // this returned null and every reply stayed unread forever.
  assert.equal(
    resolveThreadActivityItemReadAt(
      threadReply("reply-1", "general", "root-1"),
      () => null,
      (rootId) => (rootId === "root-1" ? 400 : null),
    ),
    400,
  );
});

test("a reply's own message marker still wins when it is ahead of the thread", () => {
  assert.equal(
    resolveThreadActivityItemReadAt(
      threadReply("reply-1", "general", "root-1"),
      (contextId) => (contextId === "msg:reply-1" ? 900 : null),
      () => 400,
    ),
    900,
  );
});

test("a different thread's marker does not clear this reply", () => {
  assert.equal(
    resolveThreadActivityItemReadAt(
      threadReply("reply-1", "general", "root-1"),
      () => null,
      (rootId) => (rootId === "other-root" ? 400 : null),
    ),
    null,
  );
});

test("a reply with no resolvable root falls back to the channel-level read state", () => {
  // Malformed or root-less tags must not throw, and must not silently report
  // the item as read.
  assert.equal(
    resolveThreadActivityItemReadAt(
      { id: "reply-1", channelId: "general", tags: [] },
      (contextId) => (contextId === "general" ? 250 : null),
      () => 999,
    ),
    250,
  );
});
