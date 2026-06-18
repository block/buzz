import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadUnreadMarker } from "../../messages/lib/unreadMarker.ts";
import {
  buildDirectRepliesByParentId,
  subtreeMaxCreatedAt,
} from "./subtreeCreatedAt.ts";

// Tree:    w(100)
//          ├── deep1(400) ── deep2(500)
//          └── sib(300)
// `deep1` is the deep branch (subtree-max 500); `sib` is a shallower sibling
// whose only reply (300) is chronologically older than the deep tail.
function fixture() {
  const directReplyIdsByParentId = new Map([
    ["w", ["deep1", "sib"]],
    ["deep1", ["deep2"]],
  ]);
  const createdAtByMessageId = new Map([
    ["w", 100],
    ["deep1", 400],
    ["deep2", 500],
    ["sib", 300],
  ]);
  const replies = [
    { id: "sib", createdAt: 300 },
    { id: "deep1", createdAt: 400 },
    { id: "deep2", createdAt: 500 },
  ];
  return { directReplyIdsByParentId, createdAtByMessageId, replies };
}

test("subtreeMaxCreatedAt_branchWithDescendants_returnsDeepestCreatedAt", () => {
  const { directReplyIdsByParentId, createdAtByMessageId } = fixture();

  const result = subtreeMaxCreatedAt(
    "deep1",
    directReplyIdsByParentId,
    createdAtByMessageId,
  );

  // Includes the descendant deep2(500), not just deep1's own 400.
  assert.equal(result, 500);
});

test("subtreeMaxCreatedAt_leafBranch_returnsOwnCreatedAt", () => {
  const { directReplyIdsByParentId, createdAtByMessageId } = fixture();

  const result = subtreeMaxCreatedAt(
    "sib",
    directReplyIdsByParentId,
    createdAtByMessageId,
  );

  assert.equal(result, 300);
});

test("subtreeMaxCreatedAt_absentMessage_returnsNull", () => {
  const { directReplyIdsByParentId, createdAtByMessageId } = fixture();

  const result = subtreeMaxCreatedAt(
    "ghost",
    directReplyIdsByParentId,
    createdAtByMessageId,
  );

  // Null signals the caller to skip the read-state write.
  assert.equal(result, null);
});

// Invariant 3: expanding the deep branch advances the single monotonic frontier
// to the branch subtree-max (500), which consumes the chronologically-older
// unexpanded sibling (300) too. This is the accepted single-frontier semantic.
test("expandDeepBranch_advancesFrontierToSubtreeMax_consumesOlderSibling", () => {
  const { directReplyIdsByParentId, createdAtByMessageId, replies } = fixture();

  const frontier = subtreeMaxCreatedAt(
    "deep1",
    directReplyIdsByParentId,
    createdAtByMessageId,
  );
  const marker = computeThreadUnreadMarker(replies, frontier);

  assert.equal(frontier, 500);
  // Everything at or below 500 is read — including sib(300), never expanded.
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

// Invariant 1: the session divider is computed from the open-time frontier
// SNAPSHOT, the badge/consume from the LIVE frontier. After expand advances the
// live frontier to the subtree-max (500), the two clocks deliberately diverge:
// the live frontier reports everything consumed, while the divider — read from
// the frozen open-time snapshot (100) — stays pinned on the first unread reply.
// This is what keeps the divider from moving mid-session when you expand.
test("expandAfterOpen_dividerFromSnapshot_holds_whileLiveFrontierConsumes", () => {
  const { directReplyIdsByParentId, createdAtByMessageId, replies } = fixture();

  const openSnapshot = 100;
  const liveFrontierAfterExpand = subtreeMaxCreatedAt(
    "deep1",
    directReplyIdsByParentId,
    createdAtByMessageId,
  );

  const dividerFromSnapshot = computeThreadUnreadMarker(replies, openSnapshot);
  const consumeFromLive = computeThreadUnreadMarker(
    replies,
    liveFrontierAfterExpand,
  );

  // Divider stays on the first unread, computed against the frozen snapshot...
  assert.equal(dividerFromSnapshot.firstUnreadReplyId, "sib");
  assert.equal(dividerFromSnapshot.unreadCount, 3);
  // ...even though the live frontier has consumed the whole branch.
  assert.equal(consumeFromLive.firstUnreadReplyId, null);
});

test("buildDirectRepliesByParentId_groupsDirectRepliesByParent_inOrder", () => {
  const messages = [
    { id: "root", parentId: null, createdAt: 100 },
    { id: "r1", parentId: "root", createdAt: 200 },
    { id: "deep", parentId: "r1", createdAt: 300 },
    { id: "r2", parentId: "root", createdAt: 250 },
  ];
  const index = buildDirectRepliesByParentId(messages);
  // Only DIRECT children, in timeline order — not transitive descendants.
  assert.deepEqual(
    index.get("root")?.map((m) => m.id),
    ["r1", "r2"],
  );
  assert.deepEqual(
    index.get("r1")?.map((m) => m.id),
    ["deep"],
  );
  // A top-level message with no replies is absent (the seed/count guard).
  assert.equal(index.has("r2"), false);
});

test("buildDirectRepliesByParentId_topLevelOnly_returnsEmpty", () => {
  const messages = [{ id: "root", parentId: null, createdAt: 100 }];
  assert.equal(buildDirectRepliesByParentId(messages).size, 0);
});
