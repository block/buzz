import assert from "node:assert/strict";
import test from "node:test";

import { nextThreadBadgeFrontier } from "./threadBadgeFrontier.ts";
import { seedThreadBadgeFrontiers } from "./threadBadgeFrontier.ts";
import { buildDirectRepliesByParentId } from "./subtreeCreatedAt.ts";

const msg = (id, parentId) => ({ id, parentId });
const seedAll = () => true;
const seed = (frontiers, messages, isNotified, getReadAt) =>
  seedThreadBadgeFrontiers(
    frontiers,
    messages,
    buildDirectRepliesByParentId(messages),
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
