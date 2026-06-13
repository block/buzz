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
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false);
});

test("computeThreadReplyUnreadCounts_descendantsButNoneUnread_noBadge", () => {
  // Frontier 1000: nothing is newer, so no unread descendants anywhere.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
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
    frontierSeconds: 350,
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false);
});
