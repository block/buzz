import assert from "node:assert/strict";
import test from "node:test";

import { openPersonaConversation } from "./openPersonaConversation.ts";

function persona(overrides = {}) {
  return {
    id: "builtin:command-operations",
    displayName: "Operations Adviser",
    isActive: true,
    ...overrides,
  };
}

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "Operations Adviser",
    personaId: "builtin:command-operations",
    status: "running",
    updatedAt: "2026-07-27T10:00:00Z",
    ...overrides,
  };
}

function dependencies(overrides = {}) {
  const calls = [];
  const activePersona = persona();
  const createdAgent = agent({ pubkey: "c".repeat(64) });
  return {
    calls,
    definitions: [activePersona],
    managedAgents: [],
    buildInput: async (definition) => {
      calls.push(["build", definition.id]);
      return {
        name: definition.displayName,
        personaId: definition.id,
        spawnAfterCreate: true,
      };
    },
    createAgent: async (input) => {
      calls.push(["create", input]);
      return {
        agent: createdAgent,
        privateKeyNsec: "test-only",
        profileSyncError: null,
        spawnError: null,
      };
    },
    startAgent: async (pubkey) => {
      calls.push(["start", pubkey]);
    },
    openDm: async (pubkeys) => {
      calls.push(["dm", pubkeys]);
      return { id: "dm-command-operations" };
    },
    navigate: async (channelId) => {
      calls.push(["navigate", channelId]);
    },
    refetch: async () => {
      calls.push(["refetch"]);
    },
    ...overrides,
  };
}

test("creates an uninstantiated adviser and opens its DM", async () => {
  const deps = dependencies();

  const result = await openPersonaConversation(
    "builtin:command-operations",
    deps,
  );

  assert.equal(result.pubkey, "c".repeat(64));
  assert.deepEqual(
    deps.calls.map(([kind]) => kind),
    ["build", "create", "refetch", "dm", "navigate"],
  );
  assert.deepEqual(deps.calls.at(-2), ["dm", ["c".repeat(64)]]);
});

test("reuses the newest running adviser without creating or starting", async () => {
  const older = agent({
    pubkey: "1".repeat(64),
    updatedAt: "2026-07-27T09:00:00Z",
  });
  const newest = agent({
    pubkey: "2".repeat(64),
    updatedAt: "2026-07-27T11:00:00Z",
  });
  const deps = dependencies({ managedAgents: [older, newest] });

  const result = await openPersonaConversation(
    "builtin:command-operations",
    deps,
  );

  assert.equal(result.pubkey, "2".repeat(64));
  assert.deepEqual(
    deps.calls.map(([kind]) => kind),
    ["dm", "navigate"],
  );
});

test("starts a stopped reusable adviser instead of creating a duplicate", async () => {
  const stopped = agent({ status: "stopped" });
  const deps = dependencies({ managedAgents: [stopped] });

  await openPersonaConversation("builtin:command-operations", deps);

  assert.deepEqual(
    deps.calls.map(([kind]) => kind),
    ["start", "refetch", "dm", "navigate"],
  );
  assert.deepEqual(deps.calls[0], ["start", stopped.pubkey]);
});

test("does not open a DM when the created adviser failed to spawn", async () => {
  const deps = dependencies({
    createAgent: async (input) => {
      deps.calls.push(["create", input]);
      return {
        agent: agent({ pubkey: "f".repeat(64), status: "stopped" }),
        privateKeyNsec: "test-only",
        profileSyncError: null,
        spawnError: "LM Studio is unavailable",
      };
    },
  });

  await assert.rejects(
    openPersonaConversation("builtin:command-operations", deps),
    /LM Studio is unavailable/,
  );
  assert.deepEqual(
    deps.calls.map(([kind]) => kind),
    ["build", "create", "refetch"],
  );
});

test("rejects an inactive or missing definition before mutating state", async () => {
  const deps = dependencies({
    definitions: [persona({ isActive: false })],
  });

  await assert.rejects(
    openPersonaConversation("builtin:command-operations", deps),
    /not active/,
  );
  assert.deepEqual(deps.calls, []);
});
