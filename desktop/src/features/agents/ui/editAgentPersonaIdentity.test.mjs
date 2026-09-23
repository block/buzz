import assert from "node:assert/strict";
import test from "node:test";

import {
  buildPersonaIdentityUpdate,
  isPersonaIdentityEditable,
  resolveInstanceSystemPromptUpdate,
} from "./editAgentPersonaIdentity.ts";

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "JM",
    avatarUrl: null,
    description: "Old description",
    systemPrompt: "Old instructions",
    runtime: "claude",
    model: "opus",
    provider: "anthropic",
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    sourceTeam: null,
    envVars: { A: "1" },
    respondTo: "owner-only",
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

test("isPersonaIdentityEditable rejects built-in and team personas", () => {
  assert.equal(isPersonaIdentityEditable(persona()), true);
  assert.equal(isPersonaIdentityEditable(persona({ isBuiltIn: true })), false);
  assert.equal(
    isPersonaIdentityEditable(persona({ sourceTeam: "team-1" })),
    false,
  );
  assert.equal(isPersonaIdentityEditable(null), false);
});

test("buildPersonaIdentityUpdate returns null when unchanged", () => {
  const p = persona();
  assert.equal(
    buildPersonaIdentityUpdate({
      descriptionDraft: "Old description",
      persona: p,
      systemPromptDraft: "Old instructions",
    }),
    null,
  );
});

test("buildPersonaIdentityUpdate includes description and instructions", () => {
  const p = persona();
  const input = buildPersonaIdentityUpdate({
    descriptionDraft: "New description",
    persona: p,
    systemPromptDraft: "New instructions",
  });
  assert.ok(input);
  assert.equal(input.description, "New description");
  assert.equal(input.systemPrompt, "New instructions");
  assert.equal(input.displayName, "JM");
  assert.equal(input.runtime, "claude");
  assert.deepEqual(input.envVars, { A: "1" });
});

test("resolveInstanceSystemPromptUpdate mirrors synced definition writes", () => {
  assert.equal(
    resolveInstanceSystemPromptUpdate({
      agentSystemPrompt: "Old",
      linkedPersona: persona(),
      syncedSystemPrompt: "New",
      systemPromptDraft: "New",
    }),
    "New",
  );
  assert.equal(
    resolveInstanceSystemPromptUpdate({
      agentSystemPrompt: "Same",
      linkedPersona: persona(),
      syncedSystemPrompt: "Same",
      systemPromptDraft: "Same",
    }),
    undefined,
  );
  assert.equal(
    resolveInstanceSystemPromptUpdate({
      agentSystemPrompt: "Old",
      linkedPersona: persona(),
      syncedSystemPrompt: undefined,
      systemPromptDraft: "Ignored for linked",
    }),
    undefined,
  );
  assert.equal(
    resolveInstanceSystemPromptUpdate({
      agentSystemPrompt: "Old",
      linkedPersona: null,
      syncedSystemPrompt: undefined,
      systemPromptDraft: "  Unlinked  ",
    }),
    "Unlinked",
  );
});
