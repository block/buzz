import assert from "node:assert/strict";
import test from "node:test";

import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";
import { nextThreadBadgeFrontier } from "./threadBadgeFrontier.ts";
import {
  buildCreatedAtByMessageId,
  buildDirectRepliesByParentId,
  buildDirectReplyIdsByParentId,
  subtreeMaxCreatedAt,
} from "./subtreeCreatedAt.ts";

// End-to-end model of the mark-read-on-thread-open pipeline in
// useChannelUnreadState. The open effect computes a read ceiling for the thread
// head, markThreadRead advances the thread-OWN marker toward it (monotonic, per
// advanceContext), the badge frontier snapshot then advances toward that live
// marker (seedThreadBadgeFrontiers -> nextThreadBadgeFrontier), and the
// summary badge counts the whole subtree against that snapshot
// (computeThreadBadgeCounts). The fix changed the open ceiling from the
// direct-replies max (head + direct children) to the full-subtree max
// (subtreeMaxCreatedAt); these tests pin that the badge collapses to 0 on open
// whether or not the OWN marker actually advances.

const msg = (id, parentId, createdAt, pubkey = "author") => ({
  id,
  parentId,
  createdAt,
  pubkey,
});

// The ceiling the open effect now writes: full-subtree max over the head.
const openCeiling = (rootId, messages) =>
  subtreeMaxCreatedAt(
    rootId,
    buildDirectReplyIdsByParentId(messages),
    buildCreatedAtByMessageId(messages),
  );

// Drive one thread-open through the pipeline. `priorOwnMarker` is the thread's
// OWN read marker before this open (null = never read). Returns the resulting
// badge count for the root after open.
const badgeAfterOpen = (rootId, messages, priorOwnMarker, currentPubkey) => {
  const ceiling = openCeiling(rootId, messages);
  // markThreadRead -> advanceContext: monotonic max of prior own marker and the
  // new ceiling. A null ceiling means no replies; the effect early-returns.
  const liveMarker =
    ceiling === null
      ? priorOwnMarker
      : priorOwnMarker === null
        ? ceiling
        : Math.max(priorOwnMarker, ceiling);
  // seedThreadBadgeFrontiers advances the snapshot toward the live marker.
  const frontier = nextThreadBadgeFrontier(undefined, liveMarker);
  return computeThreadBadgeCounts(
    messages,
    buildDirectRepliesByParentId(messages),
    new Map([[rootId, frontier]]),
    () => true,
    currentPubkey,
  ).get(rootId);
};

test("openThreadWithUnreadNestedReply_advancesFrontierToSubtreeMax", () => {
  // root -> a(100) -> b(200): the unread lives in nested reply b.
  // The OLD direct-replies ceiling stopped at a(100) (b is a grandchild, not a
  // direct reply of root); only subtree-max reaches the nested b(200).
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100),
    msg("b", "a", 200),
  ];
  const ids = buildDirectReplyIdsByParentId(messages);
  const createdAt = buildCreatedAtByMessageId(messages);
  assert.equal(subtreeMaxCreatedAt("root", ids, createdAt), 200);
});

test("openThreadWithUnreadNestedReply_collapsesBadgeToZero", () => {
  // The reported bug: before the fix the frontier sat at the direct-replies
  // ceiling (100) and the nested reply b(200) kept the badge lit. The fix
  // advances to subtree-max (200), so the badge recomputes to 0 on open.
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100),
    msg("b", "a", 200),
  ];
  assert.equal(badgeAfterOpen("root", messages, null), undefined);
});

test("openThreadWithUnreadNestedReply_oldDirectCeilingLeftBadgeLit", () => {
  // Regression guard: the OLD behavior advanced the frontier only to the
  // direct-replies ceiling — max over root(50) and its DIRECT reply a(100),
  // i.e. 100. The nested grandchild b(200) was excluded, so the badge stayed
  // lit (count=1). This pins the exact gap the fix closes: had the fix been
  // reverted to that ceiling, the badge would NOT clear. The subtree-max
  // ceiling (200) is asserted to clear the badge in the test above.
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100),
    msg("b", "a", 200),
  ];
  const oldDirectCeiling = 100;
  const frontier = nextThreadBadgeFrontier(undefined, oldDirectCeiling);
  const count = computeThreadBadgeCounts(
    messages,
    buildDirectRepliesByParentId(messages),
    new Map([["root", frontier]]),
    () => true,
  ).get("root");
  assert.equal(count, 1);
});

test("ownMarkerAlreadyAtSubtreeMax_stillCollapsesBadgeToZero", () => {
  // Prior-session expand synced the OWN marker to subtree-max BEFORE this
  // session's first open. markThreadRead's advance is then a no-op
  // (advanceContext early-returns, no notify), but the badge still reads 0
  // because the frontier snapshot is seeded from the live marker on render,
  // independent of whether the advance notified. Pins the no-op-return path.
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100),
    msg("b", "a", 200),
  ];
  assert.equal(badgeAfterOpen("root", messages, 200), undefined);
});

test("openThreadWhereOnlyUnreadIsOwnReply_neverShowsBadge", () => {
  // Will "commented back": a nested reply authored by the current user. Self
  // authored replies are excluded from the count, so no badge ever shows and
  // the fix is inert — the badge is already absent before and after open.
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100, "other"),
    msg("b", "a", 200, "ME"),
  ];
  // Frontier below every reply (never read) — only "other"'s reply a counts.
  const beforeOpen = computeThreadBadgeCounts(
    messages,
    buildDirectRepliesByParentId(messages),
    new Map([["root", null]]),
    () => true,
    "me",
  ).get("root");
  assert.equal(beforeOpen, 1);
  // After open the frontier reaches subtree-max (200), clearing a as well.
  assert.equal(badgeAfterOpen("root", messages, null, "me"), undefined);
});

test("openThreadWhereEveryUnreadIsOwnReply_inertNoBadgeEver", () => {
  // Every reply is the user's own → no badge before OR after open.
  const messages = [
    msg("root", null, 50),
    msg("a", "root", 100, "ME"),
    msg("b", "a", 200, "ME"),
  ];
  const before = computeThreadBadgeCounts(
    messages,
    buildDirectRepliesByParentId(messages),
    new Map([["root", null]]),
    () => true,
    "me",
  ).get("root");
  assert.equal(before, undefined);
  assert.equal(badgeAfterOpen("root", messages, null, "me"), undefined);
});
