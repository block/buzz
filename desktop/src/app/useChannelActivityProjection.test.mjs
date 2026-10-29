import assert from "node:assert/strict";
import test from "node:test";

import { resolveChannelActivityFeedItemReadAt } from "./useChannelActivityProjection.ts";

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
