import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";
import {
  nextThreadBadgeFrontier,
  seedThreadBadgeFrontiers,
} from "./threadBadgeFrontier.ts";
import { buildRepliesByRootId } from "./subtreeCreatedAt.ts";

// LP4 characterization invariants for thread-unread badges.
//
// Each invariant pins a contract the badge pipeline holds TODAY and that the
// redesign (re-key the roll-up + frontier on rootId, lift the orphan-only-root
// seed skip) must preserve. They are green on current code and stay green after
// the collapse: a redesign that changes any one of (a)-(g) has broken behavior
// Will depends on, not just refactored an internal walk.
//
// Fixtures carry `rootId` alongside `parentId` so they remain falsifiable once
// the pipeline re-keys on rootId — a rootId-keyed implementation that ignored
// parentId, or a parentId-keyed one that ignored rootId, must still satisfy the
// same observable counts here. `rootId` defaults to the thread root for the
// happy-path fixtures; the orphan/sibling defect tests live in the dedicated
// _DEFECT suites and threadOpenCeiling.test.mjs.

const msg = (id, parentId, rootId, createdAt = 100, pubkey = "author") => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
  pubkey,
});

const notifiedAll = () => true;
const counts = (messages, frontiers, isNotified = notifiedAll, currentPubkey) =>
  computeThreadBadgeCounts(
    messages,
    buildRepliesByRootId(messages),
    frontiers,
    isNotified,
    currentPubkey,
  );

const seedAll = () => true;
const seed = (frontiers, messages, getReadAt, isNotified = seedAll) =>
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    buildRepliesByRootId(messages),
    isNotified,
    getReadAt,
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
  const result = counts(messages, undefined);
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
  const result = counts(messages, undefined, (id) => id === "root1");
  assert.equal(result.get("root1"), 1);
  assert.equal(result.has("root2"), false);
});

// (c) The frontier is the read boundary: replies at or below it are read and do
// NOT count; only replies strictly newer than the frontier raise the badge.
test("invariant_c_frontierExcludesReadReplies", () => {
  const messages = [
    msg("root", null, "root", 50),
    msg("read", "root", "root", 100),
    msg("unread", "root", "root", 200),
  ];
  const frontiers = new Map([["root", 100]]);
  assert.equal(counts(messages, frontiers).get("root"), 1);
});

// (d) The current user's own replies never count as unread, at any depth.
test("invariant_d_selfAuthoredRepliesNeverUnread", () => {
  const messages = [
    msg("root", null, "root", 50, "other"),
    msg("a", "root", "root", 100, "other"),
    msg("mine", "a", "root", 200, "me"),
  ];
  assert.equal(counts(messages, undefined, notifiedAll, "me").get("root"), 1);
});

// (e) A notified root with no unread content produces NO entry — absence, not a
// zero. (The badge UI keys off presence; a 0 entry would render a phantom dot.)
test("invariant_e_noUnreadMeansNoEntry", () => {
  const messages = [
    msg("root", null, "root", 50),
    msg("a", "root", "root", 100),
  ];
  const frontiers = new Map([["root", 100]]);
  const result = counts(messages, frontiers);
  assert.equal(result.has("root"), false);
});

// (f) Seed is monotonic and frozen-at-open: once advanced toward a live marker,
// a later stale (lower) marker never lowers the snapshot. This is what keeps a
// badge from flickering back after a read, and what the redesign's rootId
// re-key must not regress.
test("invariant_f_seedMonotonicNeverLowers", () => {
  assert.equal(nextThreadBadgeFrontier(undefined, null), null); // unseeded
  assert.equal(nextThreadBadgeFrontier(null, 200), 200); // first read advances
  assert.equal(nextThreadBadgeFrontier(200, 100), 200); // stale marker held
  assert.equal(nextThreadBadgeFrontier(200, null), 200); // null never lowers
});

// (g) FALSIFIABLE LOCK — two distinct roots keep INDEPENDENT frontiers and
// badges; reading one never collapses the other. The redesign re-keys on
// rootId; if that re-key ever conflated two roots' frontiers (e.g. keyed on a
// shared channel id, or folded sibling roots into one bucket), this fails.
// Concretely: root1 read up to its newest reply (badge clears), root2 unread.
// A correct pipeline shows root2 only; a collapsing bug shows neither or both.
test("invariant_g_distinctRootsDoNotCollapse", () => {
  const messages = [
    msg("root1", null, "root1", 10),
    msg("r1reply", "root1", "root1", 100),
    msg("root2", null, "root2", 20),
    msg("r2reply", "root2", "root2", 200),
  ];
  // root1 read through its reply (frontier 100); root2 never read (null).
  const frontiers = new Map([
    ["root1", 100],
    ["root2", null],
  ]);
  const result = counts(messages, frontiers);
  assert.equal(result.has("root1"), false); // root1 fully read — no badge
  assert.equal(result.get("root2"), 1); // root2 independently still unread
  assert.equal(result.size, 1);
});

// (g) seed companion — seeding one root's frontier leaves the other untouched,
// so the two-frontier independence holds through the seed path, not only the
// count path.
test("invariant_g_seedOneRootLeavesOtherUntouched", () => {
  const frontiers = new Map();
  const messages = [
    msg("root1", null, "root1", 10),
    msg("r1reply", "root1", "root1", 100),
    msg("root2", null, "root2", 20),
    msg("r2reply", "root2", "root2", 200),
  ];
  seed(frontiers, messages, (id) => (id === "root1" ? 100 : null));
  assert.equal(frontiers.get("root1"), 100);
  assert.equal(frontiers.get("root2"), null);
  assert.equal(frontiers.size, 2);
});
