import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";
import { buildDirectRepliesByParentId } from "./subtreeCreatedAt.ts";

// Minimal TimelineMessage shape the badge counter reads: id, parentId,
// createdAt, pubkey. createdAt defaults high so replies count unread against a
// null frontier unless a test sets it lower.
const msg = (id, parentId, createdAt = 100, pubkey = "author") => ({
  id,
  parentId,
  createdAt,
  pubkey,
});

const countAll = () => true;
const counts = (messages, frontiers, isNotified = countAll, currentPubkey) =>
  computeThreadBadgeCounts(
    messages,
    buildDirectRepliesByParentId(messages),
    frontiers,
    isNotified,
    currentPubkey,
  );

test("computeThreadBadgeCounts_directRepliesOnly_countsEach", () => {
  const messages = [msg("root", null), msg("a", "root"), msg("b", "root")];
  assert.equal(counts(messages, undefined).get("root"), 2);
});

test("computeThreadBadgeCounts_nestedReply_countsTowardRoot", () => {
  // root -> a -> b: b is a reply-to-a-reply. Pre-fix it lived under a's key
  // and was never tallied toward root; the subtree walk must count it.
  const messages = [msg("root", null), msg("a", "root"), msg("b", "a")];
  assert.equal(counts(messages, undefined).get("root"), 2);
});

test("computeThreadBadgeCounts_deepChain_countsWholeSubtree", () => {
  // root -> a -> b -> c -> d: every descendant tallies toward the root.
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a"),
    msg("c", "b"),
    msg("d", "c"),
  ];
  assert.equal(counts(messages, undefined).get("root"), 4);
});

test("computeThreadBadgeCounts_branchingSubtree_countsAllBranches", () => {
  // root -> a -> {b, c}; root -> d. Four descendants across two branches.
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a"),
    msg("c", "a"),
    msg("d", "root"),
  ];
  assert.equal(counts(messages, undefined).get("root"), 4);
});

test("computeThreadBadgeCounts_rootWithNoReplies_omitted", () => {
  const messages = [msg("root", null)];
  assert.equal(counts(messages, undefined).has("root"), false);
});

test("computeThreadBadgeCounts_notNotified_omitted", () => {
  const messages = [msg("root", null), msg("a", "root"), msg("b", "a")];
  assert.equal(counts(messages, undefined, () => false).size, 0);
});

test("computeThreadBadgeCounts_frontierCoversNestedReplies_excludesRead", () => {
  // Frontier 150: a (100) is read, only nested b (200) remains unread.
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 200),
  ];
  const frontiers = new Map([["root", 150]]);
  assert.equal(counts(messages, frontiers).get("root"), 1);
});

test("computeThreadBadgeCounts_frontierCoversWholeSubtree_omitsRoot", () => {
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 120),
  ];
  const frontiers = new Map([["root", 150]]);
  assert.equal(counts(messages, frontiers).has("root"), false);
});

test("computeThreadBadgeCounts_selfAuthoredNestedReply_notCounted", () => {
  // A nested reply authored by the current user never counts as unread.
  const messages = [
    msg("root", null),
    msg("a", "root", 100, "other"),
    msg("b", "a", 200, "ME"),
  ];
  assert.equal(counts(messages, undefined, countAll, "me").get("root"), 1);
});

test("computeThreadBadgeCounts_multipleRoots_eachCountsOwnSubtree", () => {
  const messages = [
    msg("root1", null),
    msg("a", "root1"),
    msg("b", "a"),
    msg("root2", null),
    msg("c", "root2"),
  ];
  const result = counts(messages, undefined);
  assert.equal(result.get("root1"), 2);
  assert.equal(result.get("root2"), 1);
});
