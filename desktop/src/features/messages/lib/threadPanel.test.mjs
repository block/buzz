import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDescendantStatsByMessageId,
  buildMainTimelineEntries,
  buildThreadPanelData,
  buildThreadPanelDataFromIndex,
  buildThreadPanelIndex,
  buildThreadSummaryFromVisibleEntries,
  hasNestedThreadBranches,
  shouldRenderUnreadDivider,
} from "./threadPanel.ts";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";

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

test("buildMainTimelineEntries keeps huddle thread replies out of the parent timeline summary", () => {
  const huddleRoot = message({
    id: "huddle-root",
    kind: KIND_HUDDLE_STARTED,
    createdAt: 1,
    body: JSON.stringify({
      ephemeral_channel_id: "8d764100-fd8f-44cf-9c98-6d8fbd739b8c",
    }),
  });
  const reply = message({
    id: "huddle-thread-reply",
    createdAt: 2,
    parentId: "huddle-root",
    rootId: "huddle-root",
    depth: 1,
    tags: [["e", "huddle-root", "", "reply"]],
  });

  const entries = buildMainTimelineEntries([huddleRoot, reply]);

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.message.id,
      replyCount: entry.summary?.replyCount ?? 0,
    })),
    [{ id: "huddle-root", replyCount: 0 }],
  );

  const panelData = buildThreadPanelData(
    [huddleRoot, reply],
    "huddle-root",
    "huddle-root",
    new Set(),
  );
  assert.deepEqual(
    panelData.visibleReplies.map((entry) => entry.message.id),
    ["huddle-thread-reply"],
  );
});

test("buildThreadPanelData flattens every descendant into the thread panel", () => {
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
      summary: entry.summary,
    })),
    [
      { id: "direct-comment", depth: 1, summary: null },
      { id: "nested-reply", depth: 1, summary: null },
    ],
  );
});

test("buildThreadPanelData does not hide nested replies behind collapsed summaries", () => {
  const root = message({ id: "root", createdAt: 1 });
  const branch = message({
    id: "branch",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const child = message({
    id: "child",
    createdAt: 3,
    parentId: "branch",
    rootId: "root",
    depth: 2,
    tags: [
      ["e", "root", "", "root"],
      ["e", "branch", "", "reply"],
    ],
  });

  const collapsed = buildThreadPanelData(
    [root, branch, child],
    "root",
    "root",
    new Set(),
  );
  const expanded = buildThreadPanelData(
    [root, branch, child],
    "root",
    "root",
    new Set(["branch"]),
  );

  assert.deepEqual(
    collapsed.visibleReplies.map((entry) => ({
      id: entry.message.id,
      depth: entry.message.depth,
      summary: entry.summary,
    })),
    [
      { id: "branch", depth: 1, summary: null },
      { id: "child", depth: 1, summary: null },
    ],
  );
  assert.deepEqual(expanded.visibleReplies, collapsed.visibleReplies);
});

test("buildThreadSummaryFromVisibleEntries counts flattened visible rows", () => {
  const root = message({ id: "root", createdAt: 1 });
  const branch = message({
    id: "branch",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    pubkey: "branch-author",
    author: "Branch Author",
  });
  const child = message({
    id: "child",
    createdAt: 3,
    parentId: "branch",
    rootId: "root",
    depth: 2,
    pubkey: "child-author",
    author: "Child Author",
  });
  const grandchild = message({
    id: "grandchild",
    createdAt: 4,
    parentId: "child",
    rootId: "root",
    depth: 3,
    pubkey: "grandchild-author",
    author: "Grandchild Author",
  });
  const sibling = message({
    id: "sibling",
    createdAt: 5,
    parentId: "root",
    rootId: "root",
    depth: 1,
    pubkey: "sibling-author",
    author: "Sibling Author",
  });

  const collapsed = buildThreadPanelData(
    [root, branch, child, grandchild, sibling],
    "root",
    "root",
    new Set(),
  );
  const expanded = buildThreadPanelData(
    [root, branch, child, grandchild, sibling],
    "root",
    "root",
    new Set(["branch"]),
  );

  for (const entries of [collapsed.visibleReplies, expanded.visibleReplies]) {
    const summary = buildThreadSummaryFromVisibleEntries("root", entries);
    assert.equal(summary?.threadHeadId, "root");
    assert.equal(summary?.replyCount, 4);
    assert.equal(summary?.lastReplyAt, 5);
    assert.equal(summary?.participants.length, 3);
    assert.ok(
      summary?.participants.some(
        (participant) => participant.id === "sibling-author",
      ),
    );
  }
});

test("hasNestedThreadBranches returns false for flat direct replies", () => {
  const root = message({ id: "root", createdAt: 1 });
  const first = message({
    id: "first",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });
  const second = message({
    id: "second",
    createdAt: 3,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });

  const panelData = buildThreadPanelData(
    [root, first, second],
    "root",
    "root",
    new Set(),
  );

  assert.equal(hasNestedThreadBranches(panelData.visibleReplies), false);
});

test("hasNestedThreadBranches returns false for flattened nested replies", () => {
  const root = message({ id: "root", createdAt: 1 });
  const branch = message({
    id: "branch",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });
  const child = message({
    id: "child",
    createdAt: 3,
    parentId: "branch",
    rootId: "root",
    depth: 2,
  });

  const panelData = buildThreadPanelData(
    [root, branch, child],
    "root",
    "root",
    new Set(["branch"]),
  );

  assert.equal(hasNestedThreadBranches(panelData.visibleReplies), false);
});

test("hasNestedThreadBranches returns false for previously collapsed nested replies", () => {
  const root = message({ id: "root", createdAt: 1 });
  const branch = message({
    id: "branch",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
  });
  const child = message({
    id: "child",
    createdAt: 3,
    parentId: "branch",
    rootId: "root",
    depth: 2,
  });

  const panelData = buildThreadPanelData(
    [root, branch, child],
    "root",
    "root",
    new Set(),
  );

  assert.equal(hasNestedThreadBranches(panelData.visibleReplies), false);
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

// Per-id stabilization: thread rows feed `MessageRow` a depth-normalized copy
// of each reply. When `timelineMessages` churns (typing/presence) but the
// reply objects survive by reference, rebuilding the thread panel must hand
// `MessageRow` the SAME normalized object reference so the row/markdown memo
// hits — instead of a fresh `{ ...reply, depth }` spread every render.
test("thread reply objects keep identity across unrelated timelineMessages churn", () => {
  const root = message({ id: "root", createdAt: 1 });
  const replyA = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const replyB = message({
    id: "b",
    createdAt: 3,
    parentId: "a",
    rootId: "root",
    depth: 2,
    tags: [["e", "a", "", "reply"]],
  });

  // First render of the thread.
  const first = buildThreadPanelData(
    [root, replyA, replyB],
    "root",
    "root",
    new Set(["a"]),
  );

  // An unrelated channel churn produces a NEW `timelineMessages` array, but the
  // reply objects themselves are reused by reference (only their position in
  // the surrounding array changed — e.g. a presence ping or typing indicator
  // that the snapshot layer leaves the reply identities intact for).
  const churned = [
    message({ id: "noise", createdAt: 99 }),
    root,
    replyA,
    replyB,
  ];
  const second = buildThreadPanelData(churned, "root", "root", new Set(["a"]));

  const firstById = new Map(
    first.visibleReplies.map((entry) => [entry.message.id, entry.message]),
  );
  const secondById = new Map(
    second.visibleReplies.map((entry) => [entry.message.id, entry.message]),
  );

  assert.ok(firstById.size > 0, "expected at least one visible reply");
  for (const [id, normalized] of firstById) {
    assert.strictEqual(
      secondById.get(id),
      normalized,
      `normalized reply ${id} must be the SAME object reference across an unrelated churn (memo hit)`,
    );
    // Depth must still reach the row correctly via the cached object.
    assert.equal(
      typeof normalized.depth,
      "number",
      `normalized reply ${id} must carry a numeric depth`,
    );
  }
});

test("thread reply objects recompute when the source reply object is replaced", () => {
  const root = message({ id: "root", createdAt: 1 });
  const reply = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });

  const first = buildThreadPanelData([root, reply], "root", "root", new Set());

  // A genuine edit/refresh: the reply is a brand-new object (new identity).
  const editedReply = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    body: "edited body",
    tags: [["e", "root", "", "reply"]],
  });
  const second = buildThreadPanelData(
    [root, editedReply],
    "root",
    "root",
    new Set(),
  );

  const firstA = first.visibleReplies.find((e) => e.message.id === "a");
  const secondA = second.visibleReplies.find((e) => e.message.id === "a");
  assert.ok(firstA && secondA, "expected reply 'a' in both renders");
  assert.notStrictEqual(
    secondA.message,
    firstA.message,
    "a replaced source reply must produce a fresh normalized object",
  );
  assert.equal(secondA.message.body, "edited body");
});

test("buildThreadPanelDataFromIndex matches direct panel data", () => {
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
  const messages = [root, directComment, nestedReply];

  const direct = buildThreadPanelData(
    messages,
    "root",
    "direct-comment",
    new Set(["direct-comment"]),
  );
  const indexed = buildThreadPanelDataFromIndex(
    buildThreadPanelIndex(messages),
    "root",
    "direct-comment",
    new Set(["direct-comment"]),
  );

  assert.deepEqual(indexed, direct);
});
