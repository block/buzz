import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";
import { buildRepliesByRootId } from "./subtreeCreatedAt.ts";

// LP4 v3 characterization invariants for thread-unread badges.
//
// Each invariant pins a contract the per-message badge pipeline holds. They are
// the observable behaviors Will depends on: a change that breaks any one of
// (a)-(g) has regressed behavior, not just refactored an internal path.
//
// The model is per-message read markers (`msg:<id>`) read through a resolver,
// not a per-thread-root frontier snapshot. `getReadAt(id)` returns the
// effective read time for a reply (`null` = never read); a reply counts unread
// iff `createdAt > getReadAt(id)`. Reading one reply's marker never touches
// another's — independence is structural, not enforced by a separate seed.
//
// Fixtures carry `rootId` alongside `parentId` so the root-keyed roll-up stays
// falsifiable: a rootId-keyed implementation that ignored parentId, or the
// reverse, must still satisfy the same observable counts here.

const msg = (id, parentId, rootId, createdAt = 100, pubkey = "author") => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
  pubkey,
});

const notifiedAll = () => true;
const neverRead = () => null;

// A uniform read-line per thread root, applied to every reply resolving to that
// root by rootId. Translates the legacy "frontier covers part of a subtree"
// intents into the per-message resolver without a single global line.
function readLineByRoot(messages, frontiersByRoot) {
  const lineByMessageId = new Map();
  for (const message of messages) {
    const root = message.rootId ?? message.parentId ?? message.id;
    const line = frontiersByRoot.get(root);
    if (line !== undefined) lineByMessageId.set(message.id, line);
  }
  return (id) => lineByMessageId.get(id) ?? null;
}

const counts = (messages, getReadAt, isNotified = notifiedAll, currentPubkey) =>
  computeThreadBadgeCounts(
    messages,
    buildRepliesByRootId(messages),
    getReadAt,
    isNotified,
    currentPubkey,
  );

// (a) A root's badge counts EVERY descendant in its subtree, at any depth, not
// just direct replies. The whole connected subtree rolls up to one badge.
test("invariant_a_subtreeRollsUpToOneRootBadge", () => {
  const messages = [
    msg("root", null, "root"),
    msg("a", "root", "root"),
    msg("b", "a", "root"),
    msg("c", "b", "root"),
  ];
  const result = counts(messages, neverRead);
  assert.equal(result.get("root"), 3);
  assert.equal(result.size, 1);
});

// (b) Only roots the user is notified for produce a badge; an un-notified
// thread with unread replies is silent.
test("invariant_b_onlyNotifiedRootsBadge", () => {
  const messages = [
    msg("root1", null, "root1"),
    msg("a", "root1", "root1"),
    msg("root2", null, "root2"),
    msg("b", "root2", "root2"),
  ];
  const result = counts(messages, neverRead, (id) => id === "root1");
  assert.equal(result.get("root1"), 1);
  assert.equal(result.has("root2"), false);
});

// (c) The marker is the read boundary: replies at or below it are read and do
// NOT count; only replies strictly newer than the marker raise the badge.
test("invariant_c_readMarkerExcludesReadReplies", () => {
  const messages = [
    msg("root", null, "root", 50),
    msg("read", "root", "root", 100),
    msg("unread", "root", "root", 200),
  ];
  const readAt = readLineByRoot(messages, new Map([["root", 100]]));
  assert.equal(counts(messages, readAt).get("root"), 1);
});

// (d) The current user's own replies never count as unread, at any depth.
test("invariant_d_selfAuthoredRepliesNeverUnread", () => {
  const messages = [
    msg("root", null, "root", 50, "other"),
    msg("a", "root", "root", 100, "other"),
    msg("mine", "a", "root", 200, "me"),
  ];
  assert.equal(counts(messages, neverRead, notifiedAll, "me").get("root"), 1);
});

// (e) A notified root with no unread content produces NO entry — absence, not a
// zero. (The badge UI keys off presence; a 0 entry would render a phantom dot.)
test("invariant_e_noUnreadMeansNoEntry", () => {
  const messages = [
    msg("root", null, "root", 50),
    msg("a", "root", "root", 100),
  ];
  const readAt = readLineByRoot(messages, new Map([["root", 100]]));
  const result = counts(messages, readAt);
  assert.equal(result.has("root"), false);
});

// (f) PER-MESSAGE INDEPENDENCE — reading one reply's marker never clears
// another's. This is the structural fix for the original Issue 2 (an ancestor
// read covering a descendant): each reply is judged against its OWN marker, so
// reading the older reply leaves the newer one lit. A resolver that folded
// reply→reply, or keyed all replies to one shared line, would fail here.
test("invariant_f_readOneReplyLeavesOthersUnread", () => {
  const messages = [
    msg("root", null, "root", 50),
    msg("older", "root", "root", 100),
    msg("newer", "root", "root", 200),
  ];
  // Only `older` is read (marker at its own timestamp); `newer` untouched.
  const readAt = (id) => (id === "older" ? 100 : null);
  assert.equal(counts(messages, readAt).get("root"), 1);
});

// (g) FALSIFIABLE LOCK — two distinct roots keep INDEPENDENT badges; reading
// one never collapses the other (the original Face-2 cross-thread bug). root1
// read through its newest reply (badge clears), root2 unread.
test("invariant_g_distinctRootsDoNotCollapse", () => {
  const messages = [
    msg("root1", null, "root1", 10),
    msg("r1reply", "root1", "root1", 100),
    msg("root2", null, "root2", 20),
    msg("r2reply", "root2", "root2", 200),
  ];
  // root1 read through its reply (marker 100); root2 never read.
  const readAt = readLineByRoot(
    messages,
    new Map([
      ["root1", 100],
      ["root2", null],
    ]),
  );
  const result = counts(messages, readAt);
  assert.equal(result.has("root1"), false); // root1 fully read — no badge
  assert.equal(result.get("root2"), 1); // root2 independently still unread
  assert.equal(result.size, 1);
});
