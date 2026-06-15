import assert from "node:assert/strict";
import test from "node:test";

import { applyLegacyWorkspaceStorage } from "./legacyWorkspaceStorage.ts";

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

const legacyWorkspaces = JSON.stringify([
  {
    id: "legacy-workspace",
    name: "Existing relay",
    relayUrl: "wss://relay.example.com",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

const currentWorkspaces = JSON.stringify([
  {
    id: "current-workspace",
    name: "Current relay",
    relayUrl: "wss://current.example.com",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

const localhostWorkspaces = JSON.stringify([
  {
    id: "local-workspace",
    name: "Local Dev",
    relayUrl: "ws://localhost:3000",
    addedAt: "2026-06-12T00:00:00.000Z",
  },
]);

test("applyLegacyWorkspaceStorage seeds missing workspaces and active workspace", () => {
  const storage = createMemoryStorage();

  applyLegacyWorkspaceStorage(
    {
      workspaces: legacyWorkspaces,
      activeWorkspaceId: "legacy-workspace",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-workspaces"), legacyWorkspaces);
  assert.equal(storage.getItem("buzz-active-workspace-id"), "legacy-workspace");
});

test("applyLegacyWorkspaceStorage preserves existing non-local Buzz workspaces", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": currentWorkspaces,
    "buzz-active-workspace-id": "current-workspace",
  });

  applyLegacyWorkspaceStorage(
    {
      workspaces: legacyWorkspaces,
      activeWorkspaceId: "legacy-workspace",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-workspaces"), currentWorkspaces);
  assert.equal(
    storage.getItem("buzz-active-workspace-id"),
    "current-workspace",
  );
});

test("applyLegacyWorkspaceStorage replaces broken localhost first-run workspace", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": localhostWorkspaces,
    "buzz-active-workspace-id": "local-workspace",
  });

  applyLegacyWorkspaceStorage(
    {
      workspaces: legacyWorkspaces,
      activeWorkspaceId: "legacy-workspace",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-workspaces"), legacyWorkspaces);
  assert.equal(storage.getItem("buzz-active-workspace-id"), "legacy-workspace");
});

test("applyLegacyWorkspaceStorage treats trailing-slash localhost as broken", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": JSON.stringify([
      {
        id: "local-workspace",
        name: "Local Dev",
        relayUrl: "ws://localhost:3000/",
        addedAt: "2026-06-12T00:00:00.000Z",
      },
    ]),
    "buzz-active-workspace-id": "local-workspace",
  });

  applyLegacyWorkspaceStorage(
    {
      workspaces: legacyWorkspaces,
      activeWorkspaceId: "legacy-workspace",
      onboardingCompletions: [],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-workspaces"), legacyWorkspaces);
  assert.equal(storage.getItem("buzz-active-workspace-id"), "legacy-workspace");
});

test("applyLegacyWorkspaceStorage migrates onboarding completion keys", () => {
  const storage = createMemoryStorage();

  applyLegacyWorkspaceStorage(
    {
      workspaces: null,
      activeWorkspaceId: null,
      onboardingCompletions: [{ pubkey: "abc123", value: "true" }],
    },
    storage,
  );

  assert.equal(storage.getItem("buzz-onboarding-complete.v1:abc123"), "true");
});
