import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDescendantStatsByMessageId,
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

function spine(ids) {
  // root -> ids[0] -> ids[1] -> ... each a single-child reply of the previous.
  return ids.map((id, index) =>
    message({
      id,
      createdAt: index + 2,
      parentId: index === 0 ? "root" : ids[index - 1],
      rootId: "root",
      depth: index + 1,
    }),
  );
}

function unreadCounts(messages, unreadReplyIds) {
  const stats = buildDescendantStatsByMessageId(messages, unreadReplyIds);
  return Object.fromEntries(
    [...stats].map(([id, stat]) => [id, stat.unreadDescendantCount]),
  );
}

test("buildDescendantStatsByMessageId_deepUnreadUnderReadParent_bubblesToEveryAncestor", () => {
  // root -> r1 -> r2 -> r3 -> r4, only the deepest reply (r4) is unread.
  // The count must surface on every ancestor on the spine, not just r4's
  // parent — this is the "deep unread under read parents" bug.
  const root = message({ id: "root", createdAt: 1 });
  const messages = [root, ...spine(["r1", "r2", "r3", "r4"])];

  assert.deepEqual(unreadCounts(messages, new Set(["r4"])), {
    root: 1,
    r1: 1,
    r2: 1,
    r3: 1,
    r4: 0,
  });
});

test("buildDescendantStatsByMessageId_noUnreadReplies_allCountsZero", () => {
  const root = message({ id: "root", createdAt: 1 });
  const messages = [root, ...spine(["r1", "r2"])];

  assert.deepEqual(unreadCounts(messages, new Set()), {
    root: 0,
    r1: 0,
    r2: 0,
  });
});

test("buildDescendantStatsByMessageId_siblingBranches_countedIndependently", () => {
  // root has two independent branches: a (a1, unread) and b (b1, read).
  // The unread must attribute to root + a1's chain, never to the b branch.
  const root = message({ id: "root", createdAt: 1 });
  const a1 = message({
    id: "a1",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });
  const a2 = message({
    id: "a2",
    createdAt: 3,
    parentId: "a1",
    rootId: "root",
    depth: 2,
  });
  const b1 = message({
    id: "b1",
    createdAt: 4,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });

  assert.deepEqual(unreadCounts([root, a1, a2, b1], new Set(["a2"])), {
    root: 1,
    a1: 1,
    a2: 0,
    b1: 0,
  });
});

test("buildDescendantStatsByMessageId_multipleUnreadOnSpine_accumulatesOnAncestors", () => {
  // root -> r1 -> r2 -> r3, with r2 and r3 both unread. Each ancestor counts
  // every unread descendant below it, so root sees 2 and r2 sees 1.
  const root = message({ id: "root", createdAt: 1 });
  const messages = [root, ...spine(["r1", "r2", "r3"])];

  assert.deepEqual(unreadCounts(messages, new Set(["r2", "r3"])), {
    root: 2,
    r1: 2,
    r2: 1,
    r3: 0,
  });
});
