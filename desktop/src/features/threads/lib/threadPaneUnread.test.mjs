import assert from "node:assert/strict";
import test from "node:test";

import {
  firstUnreadReplyId,
  subtreeReplies,
  unreadDescendantCounts,
  unreadReplyIdsForRoot,
} from "./threadPaneUnread.ts";

const ME = "a".repeat(64);
const THEM = "b".repeat(64);

// root ─┬─ r1 ── r1a ── r1a1
//       └─ r2
const MESSAGES = [
  { id: "root", parentId: null, createdAt: 1, pubkey: THEM },
  { id: "r1", parentId: "root", createdAt: 2, pubkey: THEM },
  { id: "r1a", parentId: "r1", createdAt: 3, pubkey: ME },
  { id: "r1a1", parentId: "r1a", createdAt: 4, pubkey: THEM },
  { id: "r2", parentId: "root", createdAt: 5, pubkey: THEM },
];

test("unreadReplyIdsForRoot keeps only that thread's replies", () => {
  const ids = unreadReplyIdsForRoot(
    [
      { id: "r1", rootId: "root" },
      { id: "x1", rootId: "other" },
      { id: "root", rootId: "root" },
      { id: "r2", rootId: "root" },
    ],
    "root",
  );
  assert.deepEqual([...ids], ["r1", "r2"]);
  assert.equal(
    unreadReplyIdsForRoot([{ id: "r1", rootId: "root" }], null).size,
    0,
  );
});

test("firstUnreadReplyId follows visible order and skips your own replies", () => {
  const replies = MESSAGES.slice(1);
  assert.equal(
    firstUnreadReplyId(replies, new Set(["r2", "r1a1"]), ME),
    "r1a1",
  );
  // Your own reply never starts the unread section.
  assert.equal(
    firstUnreadReplyId(replies, new Set(["r1a", "r2"]), ME.toUpperCase()),
    "r2",
  );
  assert.equal(firstUnreadReplyId(replies, new Set(), ME), null);
});

test("unreadDescendantCounts counts unread replies anywhere beneath a row", () => {
  const counts = unreadDescendantCounts(MESSAGES, new Set(["r1a1", "r2"]));
  assert.deepEqual(Object.fromEntries(counts), { root: 2, r1: 1, r1a: 1 });
  assert.equal(unreadDescendantCounts(MESSAGES, new Set()).size, 0);
});

test("subtreeReplies covers the subtree and the message unless it is the root", () => {
  assert.deepEqual(
    subtreeReplies("r1", "root", MESSAGES).map((message) => message.id),
    ["r1", "r1a", "r1a1"],
  );
  assert.deepEqual(
    subtreeReplies("root", "root", MESSAGES).map((message) => message.id),
    ["r1", "r1a", "r1a1", "r2"],
  );
  assert.deepEqual(
    subtreeReplies("r2", "root", MESSAGES).map((message) => message.id),
    ["r2"],
  );
});
