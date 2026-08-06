import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";

async function loadSubject() {
  try {
    return await import("./usersBatchCacheSync.ts");
  } catch {
    return {};
  }
}

const ALICE = "alice-pubkey";
const BOB = "bob-pubkey";

function profile(displayName, avatarUrl) {
  return {
    displayName,
    name: null,
    avatarUrl,
    nip05Handle: null,
    ownerPubkey: null,
  };
}

test("fresh profile results update every overlapping users-batch query", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.syncUsersBatchQueryCaches, "function");

  const queryClient = new QueryClient();
  const originalUpdatedAt = 1_234;
  queryClient.setQueryData(
    ["users-batch", ALICE, BOB],
    {
      profiles: {},
      missing: [ALICE, BOB],
    },
    { updatedAt: originalUpdatedAt },
  );
  queryClient.setQueryData(["users-batch", BOB], {
    profiles: { [BOB]: profile("Bob", null) },
    missing: [],
  });

  subject.syncUsersBatchQueryCaches(queryClient, {
    profiles: { [ALICE]: profile("Alice", "https://cdn.example/alice.png") },
    missing: [],
  });

  assert.deepEqual(queryClient.getQueryData(["users-batch", ALICE, BOB]), {
    profiles: {
      [ALICE]: profile("Alice", "https://cdn.example/alice.png"),
    },
    missing: [BOB],
  });
  assert.deepEqual(queryClient.getQueryData(["users-batch", BOB]), {
    profiles: { [BOB]: profile("Bob", null) },
    missing: [],
  });
  assert.equal(
    queryClient.getQueryState(["users-batch", ALICE, BOB]).dataUpdatedAt,
    originalUpdatedAt,
  );
});

test("fresh missing results remove stale profiles from overlapping queries", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.syncUsersBatchQueryCaches, "function");

  const queryClient = new QueryClient();
  queryClient.setQueryData(["users-batch", ALICE], {
    profiles: { [ALICE]: profile("Old Alice", null) },
    missing: [],
  });

  subject.syncUsersBatchQueryCaches(queryClient, {
    profiles: {},
    missing: [ALICE.toUpperCase()],
  });

  assert.deepEqual(queryClient.getQueryData(["users-batch", ALICE]), {
    profiles: {},
    missing: [ALICE],
  });
});
