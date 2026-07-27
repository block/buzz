import assert from "node:assert/strict";
import test from "node:test";

import {
  linkedInstanceForPersonaRequest,
  personaBehaviorManagedAgentPatch,
} from "./personaBehaviorManagedAgentPatch.ts";

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "osiris",
    avatarUrl: null,
    systemPrompt: "Research carefully.",
    runtime: "goose",
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: true,
    isActive: true,
    envVars: {},
    respondTo: "allowlist",
    respondToAllowlist: ["a".repeat(64)],
    parallelism: 4,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function agent(overrides = {}) {
  return {
    pubkey: "1".repeat(64),
    name: "osiris",
    personaId: "persona-1",
    ...overrides,
  };
}

test("maps an allowlist-to-anyone definition edit to the existing instance", () => {
  assert.deepEqual(
    personaBehaviorManagedAgentPatch(
      persona(),
      persona({
        respondTo: "anyone",
        respondToAllowlist: [],
        parallelism: 8,
      }),
    ),
    {
      respondTo: "anyone",
      respondToAllowlist: [],
      parallelism: 8,
    },
  );
});

test("clearing behavior restores the instance defaults", () => {
  assert.deepEqual(
    personaBehaviorManagedAgentPatch(
      persona(),
      persona({ respondTo: null, respondToAllowlist: [], parallelism: null }),
    ),
    {
      respondTo: "owner-only",
      respondToAllowlist: [],
      parallelism: 10,
    },
  );
});

test("unrelated definition edits do not overwrite instance overrides", () => {
  assert.equal(
    personaBehaviorManagedAgentPatch(
      persona(),
      persona({ systemPrompt: "A new prompt." }),
    ),
    null,
  );
});

test("agent-management resolves one named linked instance", () => {
  const target = agent();
  assert.equal(
    linkedInstanceForPersonaRequest(
      [
        target,
        agent({ pubkey: "2".repeat(64), name: "Osiris Clone" }),
        agent({ pubkey: "3".repeat(64), personaId: "persona-2" }),
      ],
      "persona-1",
      " OSIRIS ",
    ),
    target,
  );
});

test("agent-management prefers the requesting instance without fanning out", () => {
  const first = agent({ pubkey: "1".repeat(64) });
  const requester = agent({ pubkey: "2".repeat(64) });
  assert.equal(
    linkedInstanceForPersonaRequest(
      [first, requester],
      "persona-1",
      "osiris",
      requester.pubkey.toUpperCase(),
    ),
    requester,
  );
  assert.equal(
    linkedInstanceForPersonaRequest([first, requester], "persona-1", "osiris"),
    null,
  );
});
