import assert from "node:assert/strict";
import test from "node:test";

import { seedAgentFormModel, emitAgentFormDiff } from "./agentFormModel.ts";

// ── Shared fixtures ────────────────────────────────────────────────────────────

/** Minimal AgentPersona for tests that don't need every field. */
function makeDefinition(overrides = {}) {
  return {
    id: "def-1",
    displayName: "Alice",
    avatarUrl: "",
    systemPrompt: "Be helpful.",
    runtime: "goose",
    model: "gpt-4o",
    provider: null,
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

/** Minimal ManagedAgent for tests. */
function makeInstance(overrides = {}) {
  return {
    pubkey: "pk-abc123",
    name: "Alice",
    avatarUrl: "",
    systemPrompt: "Be helpful.",
    model: null,
    provider: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    autoRestartOnConfigChange: false,
    startOnAppLaunch: false,
    ...overrides,
  };
}

// ── Regression test 1: pooled displayName materialization drop ─────────────────
//
// Spec row 1 contract: stop materializing displayName→name when the definition
// has a name pool. Pool-minted instances deliberately have different names;
// the edit coordinator must NOT emit an instance name diff when the user edits
// the definition display name but the definition has a name pool.

test("test_pooled_name_instance_name_not_materialized_when_name_pool_exists", () => {
  const definition = makeDefinition({
    displayName: "Alice",
    namePool: ["pool-instance-1", "pool-instance-2"],
  });
  const instance = makeInstance({
    // Pool-minted instance has a different name from the definition displayName.
    name: "pool-instance-1",
  });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // User edits displayName on the definition (renames the definition).
  const next = { ...saved, displayName: "Alice (renamed)" };

  const emit = emitAgentFormDiff(saved, next, ctx);

  // D-field diff: displayName changed → personaInput should carry the new name.
  assert.ok(
    emit.personaInput,
    "definition should be updated when displayName changes",
  );
  assert.equal(emit.personaInput.displayName, "Alice (renamed)");

  // I-field diff: instance name must NOT be emitted — row 1 contract prevents
  // materializing displayName→name when a name pool exists.
  assert.equal(
    emit.agentInput,
    null,
    "instance name must not be materialized when the definition has a name pool",
  );
});

test("test_pooled_name_instance_name_not_materialized_even_when_names_differ", () => {
  // The pool-instance name diverges from the definition's displayName.
  // Editing the definition displayName must still not touch the instance name.
  const definition = makeDefinition({
    displayName: "Bot",
    namePool: ["Scout", "Ranger"],
  });
  const instance = makeInstance({ name: "Scout" });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // Edit the definition displayName only.
  const next = { ...saved, displayName: "Bot (v2)" };
  const emit = emitAgentFormDiff(saved, next, ctx);

  assert.ok(emit.personaInput, "definition should be updated");
  assert.equal(
    emit.agentInput,
    null,
    "instance update must not be emitted for a pool-named agent",
  );
});

test("test_no_name_pool_explicit_instance_name_change_emits_agentInput", () => {
  // Without a name pool, explicitly editing the instanceName field should
  // produce an agentInput with the new name.
  const definition = makeDefinition({
    displayName: "Solo",
    namePool: [],
  });
  const instance = makeInstance({ name: "Solo" });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // User explicitly changes the instance name (e.g. via an instanceName field).
  const next = { ...saved, instanceName: "Solo (renamed)" };
  const emit = emitAgentFormDiff(saved, next, ctx);

  // No D-field change, so no definition update expected.
  assert.equal(
    emit.personaInput,
    null,
    "no definition update for instanceName-only change",
  );
  // I-field: instanceName changed → agentInput should carry the new name.
  assert.ok(
    emit.agentInput,
    "instance update should emit when instanceName changes",
  );
  assert.equal(emit.agentInput.name, "Solo (renamed)");
});

// ── Regression test 2: instance env wholesale-replace drop ────────────────────
//
// Spec row 8 contract: instance env edits go to the instance, NOT via a
// wholesale-replace from the definition. When the user edits a D-field
// (unrelated to env), the coordinator must NOT emit an instance envVars diff
// that would clobber per-instance env overrides.

test("test_env_clobber_dropped_when_user_edits_unrelated_d_field", () => {
  const definition = makeDefinition({
    envVars: { ANTHROPIC_API_KEY: "sk-def" },
  });
  // Instance has a per-instance env override (different from definition).
  const instance = makeInstance({
    envVars: { ANTHROPIC_API_KEY: "sk-instance-override" },
  });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // User only edits the definition displayName — does NOT touch env vars.
  const next = { ...saved, displayName: "Alice (renamed)" };
  const emit = emitAgentFormDiff(saved, next, ctx);

  // D-field diff: displayName changed.
  assert.ok(
    emit.personaInput,
    "definition should be updated when displayName changes",
  );

  // I-field: no instance env diff should be emitted (row 8 contract).
  // The "instanceEnvVars" mechanism is the only way to emit instance env;
  // the standard envVars field on the model represents the definition's env.
  // Since the user didn't touch instance env, agentInput must be null (or
  // must not contain envVars if it exists for another reason).
  if (emit.agentInput !== null) {
    assert.equal(
      emit.agentInput.envVars,
      undefined,
      "instance envVars must not be emitted when the user didn't edit instance env",
    );
  }
  // More precisely, for a definition-backed agent where only a D-field changed,
  // no instance write should be needed at all.
  assert.equal(
    emit.agentInput,
    null,
    "no instance write should occur when only a D-field changed (row 8 clobber-drop)",
  );
});

test("test_env_clobber_dropped_even_when_definition_env_differs_from_instance_env", () => {
  // Ensure the drop also holds when definition and instance envs differ:
  // the coordinator must never blindly push definition envVars onto the instance.
  const definition = makeDefinition({
    envVars: { KEY: "from-definition" },
  });
  const instance = makeInstance({
    envVars: { KEY: "from-instance" },
  });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // User edits system prompt only (D-field, no env change).
  const next = { ...saved, systemPrompt: "Updated prompt." };
  const emit = emitAgentFormDiff(saved, next, ctx);

  assert.ok(emit.personaInput, "definition should be updated");
  assert.equal(emit.agentInput, null, "instance env must not be clobbered");
});

// ── Regression test 3 (AC5): team D-field emits no personaInput ──────────────
//
// Spec pass-4 amendment: team-managed D-fields are structurally unemittable.
// emitAgentFormDiff with a definition-only context whose definition has a
// sourceTeam set must return personaInput:null even when D-fields differ.

test("test_team_definition_emits_no_personaInput_even_when_fields_differ", () => {
  // Team-managed definition: sourceTeam is set.
  const definition = makeDefinition({
    sourceTeam: "team-acme",
    displayName: "Team Bot",
    systemPrompt: "Team-managed instructions.",
  });

  const ctx = { kind: "definition-only", definition };
  const saved = seedAgentFormModel(ctx);

  // Simulate user attempting to change D-fields (should be blocked structurally,
  // but the diff function is the last defence if the UI layer fails to block it).
  const next = {
    ...saved,
    displayName: "Tampered Name",
    systemPrompt: "Tampered prompt.",
  };

  const emit = emitAgentFormDiff(saved, next, ctx);

  assert.equal(
    emit.personaInput,
    null,
    "personaInput must be null for team-managed definition-only contexts",
  );
  assert.equal(
    emit.agentInput,
    null,
    "agentInput must be null for definition-only contexts",
  );
  assert.deepEqual(
    emit.policySets,
    [],
    "policySets must be empty for definition-only contexts without an instance",
  );
});

test("test_team_definition_with_instance_emits_no_personaInput_but_allows_instance_diff", () => {
  // Team-managed definition linked to an instance: D-fields must be unemittable
  // but I/L fields (respondTo, etc.) are still editable.
  const definition = makeDefinition({
    sourceTeam: "team-acme",
    displayName: "Team Bot",
    systemPrompt: "Team-managed instructions.",
  });
  const instance = makeInstance({
    respondTo: "owner-only",
    parallelism: 1,
  });

  const ctx = { kind: "instance-with-definition", definition, instance };
  const saved = seedAgentFormModel(ctx);

  // User tries to edit a D-field AND an I-field simultaneously.
  const next = {
    ...saved,
    displayName: "Tampered Name", // D-field — team-managed, must not emit
    respondTo: "anyone", // I-field — instance-owned, must emit
  };

  const emit = emitAgentFormDiff(saved, next, ctx);

  assert.equal(
    emit.personaInput,
    null,
    "personaInput must be null even when D-field differs for team-managed definition",
  );
  assert.ok(
    emit.agentInput,
    "agentInput must be emitted when an I-field changed",
  );
  assert.equal(
    emit.agentInput.respondTo,
    "anyone",
    "agentInput should carry the updated respondTo",
  );
});
