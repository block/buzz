import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";
import { buildRepliesByRootId } from "./subtreeCreatedAt.ts";

// Minimal TimelineMessage shape the badge counter reads: id, parentId, rootId,
// createdAt, pubkey. createdAt defaults high so replies count unread against a
// never-read resolver unless a test sets it lower. `rootId` defaults to the parent
// (mirroring getThreadReference's `rootTag?.[1] ?? parentId` fallback), which is
// correct for a DIRECT reply (parent IS the root); nested replies must pass
// their true thread root explicitly, exactly as getThreadReference resolves the
// `root` e-tag that travels with every event regardless of which ancestors are
// loaded. The roll-up groups by that rootId, so a severed orphan still tallies
// at its true root.
const msg = (id, parentId, createdAt = 100, pubkey = "author", rootId) => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
  pubkey,
});

const countAll = () => true;

// LP4 v3: badges read a per-message resolver, not a per-root frontier. These
// helpers translate the legacy test intents into resolvers:
//   - `neverRead` — no message has been read (the old null-frontier case).
//   - `readLineByRoot(map)` — a uniform read-line per thread root, applied to
//     every reply that resolves to that root by rootId. Reproduces the old
//     "frontier covers part of the subtree" cases without a single global line.
const neverRead = () => null;
function readLineByRoot(messages, frontiersByRoot) {
  const lineByMessageId = new Map();
  for (const message of messages) {
    const root = message.rootId ?? message.parentId ?? message.id;
    const line = frontiersByRoot.get(root);
    if (line !== undefined) lineByMessageId.set(message.id, line);
  }
  return (id) => lineByMessageId.get(id) ?? null;
}

const counts = (messages, getReadAt, isNotified = countAll, currentPubkey) =>
  computeThreadBadgeCounts(
    messages,
    buildRepliesByRootId(messages),
    getReadAt,
    isNotified,
    currentPubkey,
  );

test("computeThreadBadgeCounts_directRepliesOnly_countsEach", () => {
  const messages = [msg("root", null), msg("a", "root"), msg("b", "root")];
  assert.equal(counts(messages, neverRead).get("root"), 2);
});

test("computeThreadBadgeCounts_nestedReply_countsTowardRoot", () => {
  // root -> a -> b: b is a reply-to-a-reply. It carries the thread root in its
  // rootId, so the root-keyed roll-up tallies it toward root, not toward a.
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a", 100, "author", "root"),
  ];
  assert.equal(counts(messages, neverRead).get("root"), 2);
});

test("computeThreadBadgeCounts_deepChain_countsWholeSubtree", () => {
  // root -> a -> b -> c -> d: every descendant carries rootId "root" and tallies
  // toward the root.
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a", 100, "author", "root"),
    msg("c", "b", 100, "author", "root"),
    msg("d", "c", 100, "author", "root"),
  ];
  assert.equal(counts(messages, neverRead).get("root"), 4);
});

test("computeThreadBadgeCounts_branchingSubtree_countsAllBranches", () => {
  // root -> a -> {b, c}; root -> d. Four descendants across two branches, all
  // carrying rootId "root".
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a", 100, "author", "root"),
    msg("c", "a", 100, "author", "root"),
    msg("d", "root"),
  ];
  assert.equal(counts(messages, neverRead).get("root"), 4);
});

test("computeThreadBadgeCounts_rootWithNoReplies_omitted", () => {
  const messages = [msg("root", null)];
  assert.equal(counts(messages, neverRead).has("root"), false);
});

test("computeThreadBadgeCounts_notNotified_omitted", () => {
  const messages = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a", 100, "author", "root"),
  ];
  assert.equal(counts(messages, neverRead, () => false).size, 0);
});

test("computeThreadBadgeCounts_readLineCoversNestedReplies_excludesRead", () => {
  // Read-line 150 across the root's subtree: a (100) is read, only nested
  // b (200) remains unread.
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 200, "author", "root"),
  ];
  const readAt = readLineByRoot(messages, new Map([["root", 150]]));
  assert.equal(counts(messages, readAt).get("root"), 1);
});

test("computeThreadBadgeCounts_readLineCoversWholeSubtree_omitsRoot", () => {
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 120, "author", "root"),
  ];
  const readAt = readLineByRoot(messages, new Map([["root", 150]]));
  assert.equal(counts(messages, readAt).has("root"), false);
});

test("computeThreadBadgeCounts_perMessageMarker_readDeepReplyKeepsSiblingBadge", () => {
  // The per-message model's defining case: marking the deep reply b read
  // leaves direct sibling a unread independently — a single subtree frontier
  // could not express "b read but a not".
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 200, "author", "root"),
  ];
  const readAt = (id) => (id === "b" ? 200 : null);
  assert.equal(counts(messages, readAt).get("root"), 1);
});

test("computeThreadBadgeCounts_forcedUnread_relightsReadReply", () => {
  // Session-local mark-unread forces a (read by its marker) back to unread.
  const messages = [
    msg("root", null),
    msg("a", "root", 100),
    msg("b", "a", 200, "author", "root"),
  ];
  const allRead = () => 1000;
  const isForcedUnread = (id) => id === "a";
  const result = computeThreadBadgeCounts(
    messages,
    buildRepliesByRootId(messages),
    allRead,
    countAll,
    undefined,
    isForcedUnread,
  );
  assert.equal(result.get("root"), 1);
});

test("computeThreadBadgeCounts_selfAuthoredNestedReply_notCounted", () => {
  // A nested reply authored by the current user never counts as unread.
  const messages = [
    msg("root", null),
    msg("a", "root", 100, "other"),
    msg("b", "a", 200, "ME", "root"),
  ];
  assert.equal(counts(messages, neverRead, countAll, "me").get("root"), 1);
});

test("computeThreadBadgeCounts_multipleRoots_eachCountsOwnSubtree", () => {
  const messages = [
    msg("root1", null),
    msg("a", "root1"),
    msg("b", "a", 100, "author", "root1"),
    msg("root2", null),
    msg("c", "root2"),
  ];
  const result = counts(messages, neverRead);
  assert.equal(result.get("root1"), 2);
  assert.equal(result.get("root2"), 1);
});

// --- LP4 Case 1: orphaned subtree from a broken parent chain rolls up ---
//
// The roll-up groups each reply under its `rootId` (buildRepliesByRootId).
// Pagination / load windows can drop an intermediate ancestor, severing the
// parent chain — but every reply still carries its true rootId (the `root`
// e-tag travels with the event, getThreadReference), so a deep reply tallies at
// its real root even when the middle ancestor is absent from the loaded array.
//
// These two tests pin the exact trigger — a missing middle ancestor — and the
// orphan-immune roll-up that counts it anyway. The third is the full-chain
// control, identical to the broken-chain result by construction.

test("computeThreadBadgeCounts_brokenParentChain_orphanedReplyRollsUpToRoot", () => {
  // Full thread is root -> a -> b -> c, but intermediate ancestor `b` is NOT in
  // the loaded array (unloaded by the timeline window). `c` is genuinely unread
  // and carries rootId "root", so the root-keyed roll-up tallies both `a` and
  // `c`: count 2, the same as if the chain were intact. The old parentId-walk
  // orphaned `c` (keyed under absent "b") and undercounted to 1.
  const loaded = [
    msg("root", null),
    msg("a", "root"),
    // msg("b", "a") — intentionally absent: unloaded intermediate ancestor.
    msg("c", "b", 100, "author", "root"),
  ];
  assert.equal(counts(loaded, neverRead).get("root"), 2);
});

test("computeThreadBadgeCounts_brokenParentChain_orphanedSoleReply_showsBadge", () => {
  // Sharper form: root's ONLY unread content is the deep reply `c`, whose
  // intermediate ancestor `b` is unloaded. `c` carries rootId "root", so the
  // roll-up still groups it under root and the badge shows count 1. The old
  // parentId-walk produced NO badge at all (root unreachable to its sole reply).
  const loaded = [
    msg("root", null),
    // msg("b", "root") — intentionally absent: unloaded intermediate ancestor.
    msg("c", "b", 100, "author", "root"),
  ];
  assert.equal(counts(loaded, neverRead).get("root"), 1);
});

test("computeThreadBadgeCounts_fullParentChain_orphanRollsUp_DESIRED", () => {
  // Control: the SAME thread with the intermediate ancestor `b` present. The
  // chain root -> a -> b -> c is intact and every descendant carries rootId
  // "root", so the root badge counts 3 — the baseline the broken-chain cases
  // above match by rolling severed orphans up by rootId.
  const loaded = [
    msg("root", null),
    msg("a", "root"),
    msg("b", "a", 100, "author", "root"),
    msg("c", "b", 100, "author", "root"),
  ];
  assert.equal(counts(loaded, neverRead).get("root"), 3);
});
