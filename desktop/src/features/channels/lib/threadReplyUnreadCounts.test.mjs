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

// LP4 v3: the panel badge reads a per-message resolver, not an open-time
// frontier snapshot, and there is no separate expanded-subtree gate — the
// per-message marker already distinguishes a read parent from its still-unread
// descendant. A uniform read-line at `seconds` (or null = never read)
// reproduces the legacy boundary cases.
const uniformReadAt = (seconds) => () => seconds;

test("computeThreadReplyUnreadCounts_collapsedBranch_countsUnreadDescendants", () => {
  // Read-line 350: a1(400), b1(500), b2(600) are unread.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    getReadAt: uniformReadAt(350),
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
    getReadAt: uniformReadAt(350),
  });
  assert.equal(counts.get("a"), 1);
  // b renders its children inline, so it carries no summary badge.
  assert.equal(counts.has("b"), false);
});

test("computeThreadReplyUnreadCounts_revealedCollapsedChild_keepsOwnSubtreeBadge", () => {
  // v3 open-at-level: expanding b reveals direct child b1 but marks only the
  // revealed set read — it does NOT clear b1's still-collapsed descendant b2.
  // The per-message marker leaves b2 unread, so the now-visible (collapsed) b1
  // carries a badge of 1. This is the deliberate reversal of the #1118
  // whole-subtree-on-open behavior.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b", "b1"],
    expandedReplyIds: new Set(["b"]),
    // b and its revealed direct child b1 are read; b2 (collapsed under b1)
    // is still unread.
    getReadAt: (id) => (id === "b1" || id === "b" ? 1000 : 350),
  });
  assert.equal(counts.get("a"), 1);
  assert.equal(counts.has("b"), false); // expanded -> no summary badge
  assert.equal(counts.get("b1"), 1); // collapsed b1 keeps its b2 badge
});

test("computeThreadReplyUnreadCounts_descendantsButNoneUnread_noBadge", () => {
  // Read-line 1000: nothing is newer, so no unread descendants anywhere.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    getReadAt: uniformReadAt(1000),
  });
  assert.equal(counts.size, 0);
});

test("computeThreadReplyUnreadCounts_neverRead_allDescendantsUnread", () => {
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    getReadAt: uniformReadAt(null),
  });
  assert.equal(counts.get("a"), 1); // a1
  assert.equal(counts.get("b"), 2); // b1, b2
});

test("computeThreadReplyUnreadCounts_otherThreadReply_notCounted", () => {
  // other1(800) is unread but outside root's subtree — its ancestor "other"
  // is not in subtreeReplyIds and must never be keyed.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b", "other"],
    expandedReplyIds: new Set(),
    getReadAt: uniformReadAt(350),
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
    getReadAt: uniformReadAt(350),
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
    getReadAt: uniformReadAt(350),
    currentPubkey: "me",
  });
  assert.equal(counts.has("a"), false); // a1 is self-authored, so 0 unread
  assert.equal(counts.get("b"), 2); // b1, b2 are by "other"
});

test("computeThreadReplyUnreadCounts_perMessageMarkers_readOneDescendantKeepsRest", () => {
  // The defining per-message case: marking b1 read leaves sibling-line b2
  // unread independently. b's badge counts only the still-unread b2.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    getReadAt: (id) => (id === "b1" || id === "a1" ? 1000 : 350),
  });
  assert.equal(counts.has("a"), false); // a1 read
  assert.equal(counts.get("b"), 1); // b1 read, only b2 remains
});

test("computeThreadReplyUnreadCounts_forcedUnread_relightsReadDescendant", () => {
  // Session-local mark-unread forces a1 (read by its marker) back to unread,
  // so collapsed parent a regains its badge.
  const counts = computeThreadReplyUnreadCounts({
    timelineMessages: fixture(),
    subtreeReplyIds: ROOT_SUBTREE,
    visibleReplyIds: ["a", "b"],
    expandedReplyIds: new Set(),
    getReadAt: uniformReadAt(1000), // everything read by marker
    isForcedUnread: (id) => id === "a1",
  });
  assert.equal(counts.get("a"), 1); // a1 forced unread
  assert.equal(counts.has("b"), false); // b subtree still read
});
