import assert from "node:assert/strict";
import test from "node:test";

import {
  getTopLevelInboxUnreadOverrideIds,
  observeOpenChannelReadState,
} from "./useChannelOpenReadState.ts";

test("opening a channel clears only its top-level Inbox overrides", () => {
  assert.deepEqual(
    getTopLevelInboxUnreadOverrideIds(
      [
        { id: "top-level", channelId: "general", tags: [] },
        {
          id: "thread-reply",
          channelId: "general",
          tags: [
            ["e", "root", "", "root"],
            ["e", "parent", "", "reply"],
          ],
        },
        { id: "other-channel", channelId: "random", tags: [] },
      ],
      "general",
    ),
    ["top-level"],
  );
});

test("a background channel does not advance source read state", () => {
  const calls = [];
  observeOpenChannelReadState({
    activeChannelId: "general",
    activeReadAt: "2026-10-09T19:00:00.000Z",
    appFocused: false,
    isChannelMember: true,
    locallyUnreadFeedItems: [{ id: "mention", channelId: "general", tags: [] }],
    markChannelRead: (...args) => calls.push(["mark", ...args]),
    undoUnread: (...args) => calls.push(["undo", ...args]),
  });

  assert.deepEqual(calls, []);
});

test("focusing the source channel consumes its top-level unread state", () => {
  const calls = [];
  observeOpenChannelReadState({
    activeChannelId: "general",
    activeReadAt: "2026-10-09T19:00:00.000Z",
    appFocused: true,
    isChannelMember: true,
    locallyUnreadFeedItems: [{ id: "mention", channelId: "general", tags: [] }],
    markChannelRead: (...args) => calls.push(["mark", ...args]),
    undoUnread: (...args) => calls.push(["undo", ...args]),
  });

  assert.deepEqual(calls, [
    ["undo", "mention"],
    ["mark", "general", "2026-10-09T19:00:00.000Z", { topLevelOnly: true }],
  ]);
});
