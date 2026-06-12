import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMainTimelineEntries,
  buildThreadPanelData,
  shouldRenderUnreadDivider,
} from "./threadPanel.ts";

function message(overrides) {
  return {
    id: "message",
    createdAt: 1,
    pubkey: "author",
    author: "Author",
    avatarUrl: null,
    role: undefined,
    personaDisplayName: undefined,
    time: "12:00 PM",
    body: "body",
    parentId: null,
    rootId: null,
    depth: 0,
    accent: false,
    pending: undefined,
    edited: false,
    kind: 9,
    tags: [],
    reactions: undefined,
    ...overrides,
  };
}

test("buildMainTimelineEntries includes broadcast replies", () => {
  const root = message({ id: "root", createdAt: 1 });
  const hiddenReply = message({
    id: "hidden-reply",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const broadcastReply = message({
    id: "broadcast-reply",
    createdAt: 3,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ],
  });

  assert.deepEqual(
    buildMainTimelineEntries([root, hiddenReply, broadcastReply]).map(
      (entry) => entry.message.id,
    ),
    ["root", "broadcast-reply"],
  );
});

test("buildThreadPanelData keeps direct comments unindented", () => {
  const root = message({ id: "root", createdAt: 1 });
  const directComment = message({
    id: "direct-comment",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const nestedReply = message({
    id: "nested-reply",
    createdAt: 3,
    parentId: "direct-comment",
    rootId: "root",
    depth: 2,
    tags: [
      ["e", "root", "", "root"],
      ["e", "direct-comment", "", "reply"],
    ],
  });

  const panelData = buildThreadPanelData(
    [root, directComment, nestedReply],
    "root",
    "root",
    new Set(["direct-comment"]),
  );

  assert.deepEqual(
    panelData.visibleReplies.map((entry) => ({
      id: entry.message.id,
      depth: entry.message.depth,
    })),
    [
      { id: "direct-comment", depth: 0 },
      { id: "nested-reply", depth: 1 },
    ],
  );
});

test("shouldRenderUnreadDivider_firstUnreadIsFirstRendered_suppressesDivider", () => {
  // Fresh/never-read channel: the first message IS the first unread, nothing
  // above it to separate from.
  assert.equal(shouldRenderUnreadDivider(0, "a", "a"), false);
});

test("shouldRenderUnreadDivider_firstUnreadMidTimeline_rendersDivider", () => {
  // Real read frontier: read messages above, unread starts at index 2.
  assert.equal(shouldRenderUnreadDivider(2, "c", "c"), true);
});

test("shouldRenderUnreadDivider_firstUnreadIsFirstOfLaterDay_rendersDivider", () => {
  // Multi-day timeline where the first unread is the first message of a later
  // day group but not the first rendered entry overall — divider still marks
  // the boundary.
  assert.equal(
    shouldRenderUnreadDivider(5, "later-day-head", "later-day-head"),
    true,
  );
});

test("shouldRenderUnreadDivider_nonMatchingEntry_noDivider", () => {
  assert.equal(shouldRenderUnreadDivider(3, "x", "y"), false);
});

test("shouldRenderUnreadDivider_noUnread_noDivider", () => {
  assert.equal(shouldRenderUnreadDivider(3, "x", null), false);
});
