import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadReplyUnreadCounts } from "./threadReplyUnreadCounts.ts";

// Open thread "root":
//   root(100)
//   ├── a(200) ── a1(400)
//   └── b(300) ── b1(500) ── b2(600)
// Sibling thread "other" lives outside root's subtree.
function fixture() {
  return [
    { id: "root", createdAt: 100, parentId: null },
    { id: "a", createdAt: 200, parentId: "root" },
    { id: "b", createdAt: 300, parentId: "root" },
    { id: "a1", createdAt: 400, parentId: "a" },
    { id: "b1", createdAt: 500, parentId: "b" },
    { id: "b2", createdAt: 600, parentId: "b1" },
    { id: "other", createdAt: 700, parentId: null },
    { id: "other1", createdAt: 800, parentId: "other" },
  ];
}

const ROOT_SUBTREE = ["a", "b", "a1", "b1", "b2"];

test("computeThreadReplyUnreadCounts_collapsedBranch_countsUnreadDescendants", () => {
  // Frontier 350: a1(400), b1(500), b2(600) are unread.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1); // a1
  assert.equal(counts.get("b"), 2); // b1, b2
});

test("computeThreadReplyUnreadCounts_expandedBranch_omitsBadge", () => {
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(["b"]),
    expandedSubtreeReplyIds: new Set(["b1", "b2"]),
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false);
});

test("computeThreadReplyUnreadCounts_expandedBranch_revealedChildNoStaleBadge", () => {
  // Expand b: mark-read-on-expand reads b's whole subtree, and the panel now
  // reveals collapsed child b1 (descendant b2 still unread vs the open-time
  // frontier). b1 must carry NO badge — the expanded subtree is excluded.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b", "b1"],
    expandedReplyIds: new Set(["b"]),
    expandedSubtreeReplyIds: new Set(["b1", "b2"]),
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false);
  assert.equal(counts.has("b1"), false);
});

test("computeThreadReplyUnreadCounts_descendantsButNoneUnread_noBadge", () => {
  // Frontier 1000: nothing is newer, so no unread descendants anywhere.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: 1000,
  });
  assert.equal(counts.size, 0);
});

test("computeThreadReplyUnreadCounts_nullFrontier_allDescendantsUnread", () => {
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: null,
  });
  assert.equal(counts.get("a"), 1); // a1
  assert.equal(counts.get("b"), 2); // b1, b2
});

test("computeThreadReplyUnreadCounts_otherThreadReply_notCounted", () => {
  // other1(800) is unread by frontier but outside root's subtree — its
  // ancestor "other" is not a visible row here and must never be keyed.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b", "other"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: 350,
  });
  assert.equal(counts.has("other"), false);
});

test("computeThreadReplyUnreadCounts_onlyVisibleRowsKeyed", () => {
  // b is collapsed and unread, but not in the visible set this render.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false);
});

test("computeThreadReplyUnreadCounts_selfAuthored_skipsOwnReplies", () => {
  // a1(400) is authored by "me" — should not count as unread.
  // b1(500) and b2(600) are authored by "other" — should count.
  const messages = [
    { id: "root", createdAt: 100, parentId: null, pubkey: "other" },
    { id: "a", createdAt: 200, parentId: "root", pubkey: "other" },
    { id: "b", createdAt: 300, parentId: "root", pubkey: "other" },
    { id: "a1", createdAt: 400, parentId: "a", pubkey: "me" },
    { id: "b1", createdAt: 500, parentId: "b", pubkey: "other" },
    { id: "b2", createdAt: 600, parentId: "b1", pubkey: "other" },
  ];
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: messages,
    subtreeReplyIds: ["a", "b", "a1", "b1", "b2"],
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
    frontierSeconds: 350,
    currentPubkey: "me",
  });
  assert.equal(counts.has("a"), false); // a1 is self-authored, so 0 unread
  assert.equal(counts.get("b"), 2); // b1, b2 are by "other"
});

test("computeThreadReplyUnreadCounts_openTimeSnapshot_survivesChannelMarkRead", () => {
  // Regression (Fix 1): the in-panel badge must reflect "what was unread when
  // the thread opened", NOT the live root marker. On channel-open
  // markChannelRead advances the channel marker to the newest TOP-LEVEL
  // message; effective(thread) = max(thread_own, channel_marker), so the live
  // value can jump PAST the nested replies and zero every badge. Passing the
  // open-time snapshot (frontier 350, captured before the advance) keeps the
  // badges; passing the post-advance live value (650, past b2(600)) loses them.
  const args = {
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    expandedSubtreeReplyIds: new Set(),
  };
  const snapshotCounts = computeThreadReplyUnreadCounts({
    ...args,
    frontierSeconds: 350,
  });
  assert.equal(snapshotCounts.get("a"), 1);
  assert.equal(snapshotCounts.get("b"), 2);

  const liveAdvancedCounts = computeThreadReplyUnreadCounts({
    ...args,
    frontierSeconds: 650,
  });
  assert.equal(liveAdvancedCounts.size, 0);
});
