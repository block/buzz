import assert from "node:assert/strict";
import test from "node:test";

import {
  instanceModelDefinitionBlockMessage,
  resolveInstanceModelDefinitionWrite,
} from "./instanceModelDefinitionWrite.ts";

// ── The instance dialog's model write, for a provider-backed linked record ───
//
// A linked record's own `model` column is never read: both spawn and provider
// deploy resolve the model through the DEFINITION (`resolve_effective_config`
// → definition → global). So the instance submit path omits `model` for a
// linked record, and for a LOCAL one that is correct — its `!model` draft
// routes to the definition dialog, which owns the write.
//
// A provider-backed record has no such second surface: the definition carries
// no backend or host command, so `agentManagementUpdateTarget` routes it to the
// instance dialog and nothing else can save its model. These tests pin the one
// decision that closes the resulting silent no-op — where a model edit goes —
// and, just as importantly, that the local path is untouched.

const persona = {
  id: "p1",
  displayName: "Marshall",
  avatarUrl: "https://example.test/marshall.png",
  systemPrompt: "Be terse.",
  runtime: "hermes",
  model: "opus-4",
  provider: "anthropic",
  namePool: ["Marshall", "Marsh"],
  isBuiltIn: false,
  isActive: true,
  sourceTeam: null,
  envVars: { ANTHROPIC_API_KEY: "sk-persona" },
  respondTo: null,
  respondToAllowlist: [],
  parallelism: null,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
};

test("a provider-backed linked record's model change writes the definition", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    model: "opus-5",
    originalModel: "opus-4",
  });

  assert.equal(result.kind, "write");
  assert.equal(result.input.id, "p1");
  assert.equal(result.input.model, "opus-5");
});

test("the definition write round-trips every field update_persona replaces wholesale", () => {
  const { input } = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    model: "opus-5",
    originalModel: "opus-4",
  });

  // `update_persona` replaces these outright; sending a partial input would
  // wipe the definition's prompt, credentials, provider or name pool as the
  // price of changing one model id.
  assert.equal(input.displayName, "Marshall");
  assert.equal(input.systemPrompt, "Be terse.");
  assert.equal(input.runtime, "hermes");
  assert.equal(input.provider, "anthropic");
  assert.deepEqual(input.namePool, ["Marshall", "Marsh"]);
  assert.deepEqual(input.envVars, { ANTHROPIC_API_KEY: "sk-persona" });
  assert.equal(input.avatarUrl, "https://example.test/marshall.png");
});

test("a LOCAL linked record writes nothing — its definition dialog is the owner", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: false,
    personaId: "p1",
    linkedPersona: persona,
    model: "opus-5",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, { kind: "none" });
});

test("a definition-less provider record writes nothing — its own column is read", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: null,
    linkedPersona: null,
    model: "opus-5",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, { kind: "none" });
});

test("an unchanged model writes nothing", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    model: "opus-4",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, { kind: "none" });
});

test("whitespace-only difference is not a change", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    model: "  opus-4  ",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, { kind: "none" });
});

test("clearing the model writes undefined — 'let the harness pick' is a real value", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    model: "",
    originalModel: "opus-4",
  });

  assert.equal(result.kind, "write");
  assert.equal(result.input.model, undefined);
});

test("setting a model on a record that had none is a change", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: { ...persona, model: null },
    model: "opus-5",
    originalModel: null,
  });

  assert.equal(result.kind, "write");
  assert.equal(result.input.model, "opus-5");
});

/**
 * The `!model` seed regression. The dialog seeds the field with the REQUESTED
 * model so the owner reviews what was asked for, so comparing against the seed
 * would read every reviewed request as "unchanged" and drop it — the exact
 * silent no-op this module exists to remove. The comparison basis is the
 * record's effective model.
 */
test("a !model prefill differing from the record's model is a change", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: persona,
    // Field seeded from the draft, untouched by the owner.
    model: "opus-5",
    // What the record actually runs.
    originalModel: "opus-4",
  });

  assert.equal(result.kind, "write");
  assert.equal(result.input.model, "opus-5");
});

test("an unresolved definition blocks rather than dropping the change", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: null,
    model: "opus-5",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, {
    kind: "blocked",
    reason: "unresolved-definition",
  });
});

test("a team-managed definition blocks — an in-app edit would not survive re-import", () => {
  const result = resolveInstanceModelDefinitionWrite({
    isProviderRecord: true,
    personaId: "p1",
    linkedPersona: { ...persona, sourceTeam: "team-1" },
    model: "opus-5",
    originalModel: "opus-4",
  });

  assert.deepEqual(result, { kind: "blocked", reason: "team-managed" });
});

test("an unresolved or team-managed definition with NO model change never blocks", () => {
  for (const linkedPersona of [null, { ...persona, sourceTeam: "team-1" }]) {
    const result = resolveInstanceModelDefinitionWrite({
      isProviderRecord: true,
      personaId: "p1",
      linkedPersona,
      model: "opus-4",
      originalModel: "opus-4",
    });
    // Renaming a team agent's instance must stay possible.
    assert.deepEqual(result, { kind: "none" });
  }
});

test("each block reason has its own message", () => {
  const team = instanceModelDefinitionBlockMessage("team-managed");
  const unresolved = instanceModelDefinitionBlockMessage(
    "unresolved-definition",
  );
  assert.notEqual(team, unresolved);
  assert.match(team, /team/i);
  assert.ok(unresolved.length > 0);
});
