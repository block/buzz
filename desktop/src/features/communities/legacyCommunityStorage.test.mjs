import assert from "node:assert/strict";
import test from "node:test";

import { applyLegacyCommunityStorage } from "./legacyCommunityStorage.ts";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    removeItem(key) {
      values.delete(key);
    },
    clear() {
      values.clear();
    },
    key(index) {
      return Array.from(values.keys())[index] ?? null;
    },
    get length() {
      return values.size;
    },
  };
}

const legacyCommunities = JSON.stringify([
  {
    id: "legacy-community",
    name: "Existing relay",
    relayUrl: "wss://relay.example.com",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

const currentCommunities = JSON.stringify([
  {
    id: "current-community",
    name: "Current relay",
    relayUrl: "wss://current.example.com",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

const localhostCommunities = JSON.stringify([
  {
    id: "local-community",
    name: "Local Dev",
    relayUrl: "ws://localhost:3000",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

test("applyLegacyCommunityStorage seeds missing communities and active community", () => {
  const storage = createMemoryStorage();

  applyLegacyCommunityStorage(
    {
      workspaces: legacyCommunities,
      activeWorkspaceId: "legacy-community",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-communities"), legacyCommunities);
  assert.equal(storage.getItem("buzz-active-community-id"), "legacy-community");
});

test("applyLegacyCommunityStorage preserves existing non-local Buzz communities", () => {
  const storage = createMemoryStorage({
    "buzz-communities": currentCommunities,
    "buzz-active-community-id": "current-community",
  });

  applyLegacyCommunityStorage(
    {
      workspaces: legacyCommunities,
      activeWorkspaceId: "legacy-community",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-communities"), currentCommunities);
  assert.equal(
    storage.getItem("buzz-active-community-id"),
    "current-community",
  );
});

test("applyLegacyCommunityStorage replaces broken localhost first-run community", () => {
  const storage = createMemoryStorage({
    "buzz-communities": localhostCommunities,
    "buzz-active-community-id": "local-community",
  });

  applyLegacyCommunityStorage(
    {
      workspaces: legacyCommunities,
      activeWorkspaceId: "legacy-community",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-communities"), legacyCommunities);
  assert.equal(storage.getItem("buzz-active-community-id"), "legacy-community");
});

test("applyLegacyCommunityStorage treats trailing-slash localhost as broken", () => {
  const storage = createMemoryStorage({
    "buzz-communities": JSON.stringify([
      {
        id: "local-community",
        name: "Local Dev",
        relayUrl: "ws://localhost:3000/",
        addedAt: "2026-06-12T00:00:00.000Z",
      },
    ]),
    "buzz-active-community-id": "local-community",
  });

  applyLegacyCommunityStorage(
    {
      workspaces: legacyCommunities,
      activeWorkspaceId: "legacy-community",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-communities"), legacyCommunities);
  assert.equal(storage.getItem("buzz-active-community-id"), "legacy-community");
});

test("applyLegacyCommunityStorage migrates onboarding completion keys", () => {
  const storage = createMemoryStorage();

  applyLegacyCommunityStorage(
    {
      workspaces: null,
      activeWorkspaceId: null,
      onboardingCompletions: [{ pubkey: "abc123", value: "true" }],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-onboarding-complete.v1:abc123"), "true");
});
