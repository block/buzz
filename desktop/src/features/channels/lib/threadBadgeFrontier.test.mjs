import assert from "node:assert/strict";
import test from "node:test";

import { nextThreadBadgeFrontier } from "./threadBadgeFrontier.ts";
import { seedThreadBadgeFrontiers } from "./threadBadgeFrontier.ts";
import { buildRepliesByRootId } from "./subtreeCreatedAt.ts";
import { computeThreadBadgeCounts } from "./threadBadgeCounts.ts";

const msg = (id, parentId) => ({ id, parentId, rootId: parentId ?? id });
const seedAll = () => true;
const seed = (frontiers, messages, isNotified, getReadAt) =>
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    buildRepliesByRootId(messages),
    isNotified,
    getReadAt,
  );

test("nextThreadBadgeFrontier_unseededNullMarker_seedsNull", () => {
  // Thread never read: snapshot seeds to null (everything unread).
  assert.equal(nextThreadBadgeFrontier(undefined, null), null);
});

test("nextThreadBadgeFrontier_unseededWithMarker_seedsToMarker", () => {
  assert.equal(nextThreadBadgeFrontier(undefined, 100), 100);
});

test("nextThreadBadgeFrontier_readAdvancesMarker_advancesSnapshot", () => {
  // Snapshot frozen at open (null), user reads → live marker 200 → badge clears.
  assert.equal(nextThreadBadgeFrontier(null, 200), 200);
});

test("nextThreadBadgeFrontier_markerNewerThanStored_advances", () => {
  assert.equal(nextThreadBadgeFrontier(100, 250), 250);
});

test("nextThreadBadgeFrontier_markerOlderThanStored_keepsStored", () => {
  // Monotonic: a stale lower marker never lowers the snapshot.
  assert.equal(nextThreadBadgeFrontier(250, 100), 250);
});

test("nextThreadBadgeFrontier_markerNullAfterSeed_keepsStored", () => {
  // Live marker reads null (never read) but snapshot already advanced — hold.
  assert.equal(nextThreadBadgeFrontier(150, null), 150);
});

test("nextThreadBadgeFrontier_markerEqualsStored_unchanged", () => {
  assert.equal(nextThreadBadgeFrontier(150, 150), 150);
});

test("nextThreadBadgeFrontier_storedNullMarkerZero_advancesToZero", () => {
  // Zero is a valid frontier (epoch); null is strictly lower than any number.
  assert.equal(nextThreadBadgeFrontier(null, 0), 0);
});

test("seedThreadBadgeFrontiers_threadWithReplies_seedsToMarker", () => {
  const frontiers = new Map();
  const messages = [msg("root", null), msg("r1", "root")];
  seed(frontiers, messages, seedAll, (id) => (id === "root" ? 100 : null));
  assert.equal(frontiers.get("root"), 100);
});

test("seedThreadBadgeFrontiers_threadWithoutReplies_skipped", () => {
  const frontiers = new Map();
  seed(frontiers, [msg("root", null)], seedAll, () => 100);
  assert.equal(frontiers.has("root"), false);
});

test("seedThreadBadgeFrontiers_notNotified_skipped", () => {
  const frontiers = new Map();
  const messages = [msg("root", null), msg("r1", "root")];
  seed(
    frontiers,
    messages,
    () => false,
    () => 100,
  );
  assert.equal(frontiers.has("root"), false);
});

test("seedThreadBadgeFrontiers_replyEntry_neverSeeded", () => {
  // A reply is never a badge root even if its id collides with a notified set.
  const frontiers = new Map();
  const messages = [msg("r1", "root"), msg("r2", "root")];
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.size, 0);
});

test("seedThreadBadgeFrontiers_reseed_advancesMonotonically", () => {
  const frontiers = new Map([["root", 100]]);
  const messages = [msg("root", null), msg("r1", "root")];
  // Re-render after the live marker advanced to 250 on read.
  seed(frontiers, messages, seedAll, () => 250);
  assert.equal(frontiers.get("root"), 250);
  // A stale lower marker never lowers an already-advanced snapshot.
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 250);
});

// --- LP4 Case 3 demonstration: seed-vs-mark-read race poisons the frontier ---
//
// seedThreadBadgeFrontiers seeds each root via getReadAt(root), which resolves
// to the EFFECTIVE thread marker = max(thread_own_marker, channel_marker).
// On channel open, markChannelRead advances the channel marker to the newest
// top-level message. If that fold lands before (or in) the render where a root
// is first seeded, the seed adopts a frontier PAST the unread reply, and
// computeThreadBadgeCounts then reads zero unread — the badge vanishes
// everywhere. What the seed READS (folded vs. pre-mark-read marker) is the sole
// determinant; the seed/compute mechanics are otherwise identical.
//
// These tests drive seed -> compute end-to-end and pass against TODAY's code.
// The first DOCUMENTS THE DEFECT (folded marker -> no badge); the second is the
// pre-mark-read control (own marker -> badge survives).

// Richer message shape than the file-level `msg`: computeThreadBadgeCounts reads
// createdAt and pubkey, which the frontier-only helper omits. rootId defaults to
// the parent (getThreadReference's fallback) so these fixtures stay falsifiable
// once the seed/count pipeline re-keys on rootId.
const reply = (id, parentId, createdAt, rootId) => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
  pubkey: "author",
});

test("seedThreadBadgeFrontiers_channelMarkerFoldedIntoSeed_badgeVanishes_DEFECT", () => {
  // Thread "root" has one unread reply at createdAt 200. The thread's OWN read
  // marker is 100 (reply is genuinely unread). But channel-open mark-read has
  // already advanced the channel marker to 250, so the EFFECTIVE marker
  // getReadAt returns is max(100, 250) = 250.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const repliesByRootId = buildRepliesByRootId(messages);
  const foldedEffectiveMarker = Math.max(100, 250); // thread_own vs channel

  const frontiers = new Map();
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    repliesByRootId,
    seedAll,
    () => foldedEffectiveMarker,
  );
  // DEFECT: frontier seeded to 250, past the unread reply at 200.
  assert.equal(frontiers.get("root"), 250);

  const result = computeThreadBadgeCounts(
    messages,
    repliesByRootId,
    frontiers,
    seedAll,
  );
  // DEFECT: badge vanishes — no count anywhere despite a genuinely unread reply.
  assert.equal(result.has("root"), false);
});

test("seedThreadBadgeFrontiers_preMarkReadMarkerSeeded_badgeSurvives_DESIRED", () => {
  // Identical thread, but the seed reads the PRE-mark-read marker: the thread's
  // own marker (100), captured before the channel-open fold advanced it to 250.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const repliesByRootId = buildRepliesByRootId(messages);
  const preMarkReadMarker = 100; // thread_own only, channel fold not applied

  const frontiers = new Map();
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    repliesByRootId,
    seedAll,
    () => preMarkReadMarker,
  );
  // Frontier seeded to 100, behind the unread reply at 200.
  assert.equal(frontiers.get("root"), 100);

  const result = computeThreadBadgeCounts(
    messages,
    repliesByRootId,
    frontiers,
    seedAll,
  );
  // DESIRED: badge survives — the reply at 200 is correctly counted unread.
  assert.equal(result.get("root"), 1);
});

// --- LP4 Case 3, second face: orphan-only root IS seed-eligible ---
//
// seedThreadBadgeFrontiers gates seed-eligibility on a reply existing under the
// root by rootId: `if (!repliesByRootId.has(message.id)) continue;`. A root
// whose ONLY reply is a deep orphan — the middle ancestor unloaded, so the
// orphan keys under its absent parent in the direct-reply map — still owns that
// reply by rootId (getThreadReference resolves rootId from the event's own
// `root` e-tag regardless of ancestor load state). The old direct-reply gate
// skipped such a root entirely, so its frontier never existed and its badge
// could never clear; keying on rootId makes it seed-eligible. This is a face of
// Case 3 distinct from a wrong COUNT — here the frontier snapshot itself was
// missing, so even a corrected count path had nothing to measure against.

test("seedThreadBadgeFrontiers_orphanOnlyRoot_seedEligible", () => {
  // root's only reply is `c`, whose middle ancestor `b` is unloaded. `c` keys
  // under "b" (absent) in the direct-reply map but carries rootId === "root",
  // so repliesByRootId has an entry for "root" and the root seeds to marker 100.
  const messages = [
    reply("root", null, 50),
    // reply("b", "root", ...) — intentionally absent: unloaded ancestor.
    reply("c", "b", 200, "root"),
  ];
  const frontiers = new Map();
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 100);
});

test("seedThreadBadgeFrontiers_directReplyRoot_seedEligible_DESIRED", () => {
  // Control: the SAME root with a DIRECT reply present. repliesByRootId has an
  // entry for "root", so it is seed-eligible — the intact-chain baseline.
  const messages = [reply("root", null, 50), reply("r1", "root", 200)];
  const frontiers = new Map();
  seed(frontiers, messages, seedAll, () => 100);
  assert.equal(frontiers.get("root"), 100);
});
