import assert from "node:assert/strict";
import test from "node:test";

import {
  isThreadRailPinActive,
  threadRailPinToChannelNavigation,
  threadRailRootIdFromSearch,
} from "./threadRailNavigation.ts";

const PIN = {
  channelId: "channel-a",
  rootId: "root-a",
  pinnedAt: 100,
};

test("threadRailPinToChannelNavigation restores a nested return anchor under the canonical root", () => {
  assert.deepEqual(
    threadRailPinToChannelNavigation({
      ...PIN,
      returnAnchorId: "nested-reply-a",
    }),
    {
      channelId: "channel-a",
      thread: "root-a",
      messageId: "nested-reply-a",
      threadRootId: "root-a",
    },
  );
});

test("threadRailPinToChannelNavigation opens the pin canonical root", () => {
  assert.deepEqual(threadRailPinToChannelNavigation(PIN), {
    channelId: "channel-a",
    thread: "root-a",
    messageId: "root-a",
    threadRootId: "root-a",
  });
});

test("threadRailRootIdFromSearch prefers the canonical thread route", () => {
  assert.equal(
    threadRailRootIdFromSearch({ thread: "root-a", threadRootId: "legacy" }),
    "root-a",
  );
  assert.equal(
    threadRailRootIdFromSearch({ threadRootId: "legacy" }),
    "legacy",
  );
  assert.equal(threadRailRootIdFromSearch({ thread: 42 }), null);
});

test("isThreadRailPinActive matches the selected channel and open root", () => {
  assert.equal(isThreadRailPinActive(PIN, "channel-a", "root-a"), true);
  assert.equal(isThreadRailPinActive(PIN, "channel-a", "root-b"), false);
  assert.equal(isThreadRailPinActive(PIN, "channel-b", "root-a"), false);
  assert.equal(isThreadRailPinActive(PIN, "channel-a", null), false);
});
