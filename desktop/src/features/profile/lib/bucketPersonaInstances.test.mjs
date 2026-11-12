import assert from "node:assert/strict";
import test from "node:test";

import { bucketPersonaInstances } from "./useCanonicalManagedAgentProfile.ts";

const LIVE_PK = "b".repeat(64);
const SECOND_LIVE_PK = "c".repeat(64);
const ARCHIVED_PK = "a".repeat(64);

function agent(overrides = {}) {
  return {
    name: "Instance",
    pubkey: LIVE_PK,
    personaId: "persona-1",
    status: "stopped",
    ...overrides,
  };
}

test("mixed roster splits archived from live, preserving order", () => {
  const archived = agent({ pubkey: ARCHIVED_PK });
  const live = agent({ pubkey: LIVE_PK });
  const secondLive = agent({ pubkey: SECOND_LIVE_PK });

  const { live: liveBucket, archived: archivedBucket } = bucketPersonaInstances(
    [archived, live, secondLive],
    (pubkey) => pubkey === ARCHIVED_PK,
  );

  assert.deepEqual(liveBucket, [live, secondLive]);
  assert.deepEqual(archivedBucket, [archived]);
});

test("an all-archived persona puts every instance in the archived bucket", () => {
  const first = agent({ pubkey: ARCHIVED_PK });
  const second = agent({ pubkey: LIVE_PK });

  const { live, archived } = bucketPersonaInstances(
    [first, second],
    () => true,
  );

  assert.deepEqual(live, []);
  assert.deepEqual(archived, [first, second]);
});

test("fail-open while loading keeps every instance live", () => {
  // The predicate returns `false` for all pubkeys until the archive snapshot
  // loads; nothing is bucketed as archived, so nothing is labeled or hidden.
  const archived = agent({ pubkey: ARCHIVED_PK });
  const live = agent({ pubkey: LIVE_PK });

  const { live: liveBucket, archived: archivedBucket } = bucketPersonaInstances(
    [archived, live],
    () => false,
  );

  assert.deepEqual(liveBucket, [archived, live]);
  assert.deepEqual(archivedBucket, []);
});
