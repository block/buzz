import assert from "node:assert/strict";
import test from "node:test";

import {
  seedAgentFormModel,
  emitAgentFormDiff,
  fieldOwner,
  FIELD_OWNERS,
} from "./agentFormModel.ts";

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

// ── Phantom-write probes (Thufir pass-2 CRITICAL-1 corrective action) ─────────
//
// An untouched linked agent save must emit zero changes. These probe the three
// phantom writes Thufir found at 3a2806c06c:
// 1. Linked access (respondTo null vs. "anyone" seed default)
// 2. Instance parallelism serialized into definition behavior
// 3. Linked access overwrite on an untouched linked agent

test("test_untouched_linked_agent_emits_no_diff", () => {
  // Agent with null respondTo and parallelism (DB defaults).
  const definition = makeDefinition({
    displayName: "Alice",
    systemPrompt: "Be helpful.",
    respondTo: null,
    parallelism: null,
  });
  const instance = makeInstance({
    respondTo: null,
    parallelism: null,
  });
  const ctx = { kind: "instance-with-definition", definition, instance };
  const seed = seedAgentFormModel(ctx);
  // Simulate dialog seeding: respondTo null → "anyone" (normalized in seedAgentFormModel)
  // but since we now normalize in seed itself, seed.respondTo should be "anyone"
  assert.equal(
    seed.respondTo,
    "anyone",
    "seed normalizes null respondTo to anyone",
  );

  // Diff against itself — nothing changed.
  const emit = emitAgentFormDiff(seed, seed, ctx);
  assert.equal(emit.personaInput, null, "untouched linked: no D-write");
  assert.equal(emit.agentInput, null, "untouched linked: no I-write");
  assert.deepEqual(emit.policySets, [], "untouched linked: no policy writes");
});

test("test_untouched_team_linked_agent_emits_no_diff", () => {
  const definition = makeDefinition({
    sourceTeam: "team-acme",
    respondTo: null,
  });
  const instance = makeInstance({ respondTo: null, parallelism: null });
  const ctx = { kind: "instance-with-definition", definition, instance };
  const seed = seedAgentFormModel(ctx);

  const emit = emitAgentFormDiff(seed, seed, ctx);
  assert.equal(emit.personaInput, null, "untouched team: no D-write");
  assert.equal(emit.agentInput, null, "untouched team: no I-write");
});

test("test_linked_access_edit_emits_only_agentInput_not_personaInput", () => {
  // Changing respondTo on a linked agent must go to agentInput only,
  // never to personaInput (D-owned default stays untouched).
  const definition = makeDefinition({ respondTo: "anyone" });
  const instance = makeInstance({ respondTo: "anyone" });
  const ctx = { kind: "instance-with-definition", definition, instance };
  const seed = seedAgentFormModel(ctx);

  const next = { ...seed, respondTo: "owner-only" };
  const emit = emitAgentFormDiff(seed, next, ctx);

  assert.equal(emit.personaInput, null, "linked respondTo change: no D-write");
  assert.ok(
    emit.agentInput,
    "linked respondTo change: agentInput must be emitted",
  );
  assert.equal(emit.agentInput.respondTo, "owner-only");
});

test("test_untouched_parallelism_on_linked_agent_emits_no_diff", () => {
  // Parallelism is I-owned for linked agents. Seeding and re-diffing without
  // change must not emit to definition behavior.
  const definition = makeDefinition({ parallelism: 2 });
  const instance = makeInstance({ parallelism: 5 });
  const ctx = { kind: "instance-with-definition", definition, instance };
  const seed = seedAgentFormModel(ctx);

  // seed.parallelism should come from instance (5), not definition (2)
  assert.equal(seed.parallelism, 5, "parallelism seeds from instance");

  // Untouched diff.
  const emit = emitAgentFormDiff(seed, seed, ctx);
  assert.equal(emit.personaInput, null, "untouched parallelism: no D-write");
  assert.equal(emit.agentInput, null, "untouched parallelism: no I-write");
});

// ── R6 respondTo carry (Thufir pass-2 IMPORTANT-4 regression) ────────────────
//
// When an R6 agent-origin owner-review update request carries a `respondTo`
// change, the coordinator must:
//   1. Seed the definition-only form model with the overridden value.
//   2. Emit it through the coordinator as a D-owned field in `personaInput`
//      (definition-only context: no instance → respondTo goes to D).
// This tests the pure model chain: seedAgentFormModel + override applied
// (mirrors AgentEditMergedDialog's useEffect at line 294–297) + emitAgentFormDiff.
//
// Production flow:
//   useAgentManagement.reviewOverrides.respondTo → initialValueOverrides.respondTo
//   → dialog useEffect seeds setRespondTo → emitAgentFormDiff routes to personaInput.

test("test_r6_respond_to_override_seeds_form_and_emits_through_coordinator", () => {
  // A definition with current respondTo = null (DB default → "anyone").
  const definition = makeDefinition({ respondTo: null });
  const ctx = { kind: "definition-only", definition };

  // Step 1: seed the form model from the definition.
  const seed = seedAgentFormModel(ctx);

  // Step 2: apply the R6 override (mirrors the dialog's useEffect that sets
  // respondTo from initialValueOverrides.respondTo).
  const withOverride = { ...seed, respondTo: "owner-only" };

  // Step 3: emit the diff — the form changed respondTo from the seed value.
  const emit = emitAgentFormDiff(seed, withOverride, ctx);

  // For definition-only context, respondTo is D-owned → must land in personaInput.
  assert.ok(
    emit.personaInput,
    "R6 respondTo override must produce a personaInput diff",
  );
  assert.equal(
    emit.personaInput.behavior?.respondTo,
    "owner-only",
    "personaInput.behavior must carry the agent-requested respondTo",
  );
  // No instance in context → agentInput must be null.
  assert.equal(emit.agentInput, null, "definition-only context: no agentInput");
});

test("test_r6_respond_to_override_only_no_other_fields_emitted", () => {
  // A respond-to-only R6 request must NOT dirty any other field.
  const definition = makeDefinition({
    displayName: "Research bot",
    systemPrompt: "Original prompt.",
    respondTo: "anyone",
  });
  const ctx = { kind: "definition-only", definition };
  const seed = seedAgentFormModel(ctx);

  // Only respondTo changes — all other fields stay at seed values.
  const withOverride = { ...seed, respondTo: "owner-only" };
  const emit = emitAgentFormDiff(seed, withOverride, ctx);

  assert.ok(
    emit.personaInput,
    "personaInput present for respondTo-only change",
  );
  // The personaInput is a full-write snapshot — all definition fields are included.
  // The display name and system prompt are unchanged from the seed.
  assert.equal(
    emit.personaInput.displayName,
    "Research bot",
    "displayName carries the unchanged seed value in the full-write snapshot",
  );
  assert.equal(
    emit.personaInput.systemPrompt,
    "Original prompt.",
    "systemPrompt carries the unchanged seed value in the full-write snapshot",
  );
  assert.equal(emit.personaInput.behavior?.respondTo, "owner-only");
});

// ── fieldOwner load-bearing tests ─────────────────────────────────────────────
//
// Verifies that fieldOwner() is the actual routing mechanism consumed by
// emitAgentFormDiff. Each test confirms that (a) fieldOwner returns the right
// owner for a context, and (b) emitAgentFormDiff routes to the correct layer.

test("test_fieldOwner_respondTo_is_instance_in_linked_context", () => {
  const definition = makeDefinition();
  const instance = makeInstance({ respondTo: "owner-only" });
  const ctx = { kind: "instance-with-definition", definition, instance };

  // fieldOwner must resolve to "instance" for linked context.
  assert.equal(
    fieldOwner("respondTo", ctx),
    "instance",
    "respondTo is I-owned in instance-with-definition context",
  );
});

test("test_fieldOwner_respondTo_is_definition_in_definition_only_context", () => {
  const definition = makeDefinition();
  const ctx = { kind: "definition-only", definition };

  // fieldOwner must resolve to "definition" for definition-only context (rows 9–10).
  assert.equal(
    fieldOwner("respondTo", ctx),
    "definition",
    "respondTo is D-owned in definition-only context",
  );
});

test("test_fieldOwner_parallelism_follows_same_contract_as_respondTo", () => {
  const definition = makeDefinition();
  const instance = makeInstance({ parallelism: 3 });
  const linked = { kind: "instance-with-definition", definition, instance };
  const defOnly = { kind: "definition-only", definition };

  assert.equal(fieldOwner("parallelism", linked), "instance");
  assert.equal(fieldOwner("parallelism", defOnly), "definition");
});

test("test_fieldOwner_systemPrompt_is_instance_in_instance_only_context", () => {
  const instance = makeInstance();
  const ctx = { kind: "instance-only", instance };

  // systemPrompt is D-owned when a definition is present, I-owned for unlinked agents.
  assert.equal(fieldOwner("systemPrompt", ctx), "instance");
});

test("test_fieldOwner_model_and_provider_are_instance_in_instance_only_context", () => {
  const instance = makeInstance();
  const ctx = { kind: "instance-only", instance };

  assert.equal(fieldOwner("model", ctx), "instance");
  assert.equal(fieldOwner("provider", ctx), "instance");
});

test("test_fieldOwner_all_definition_fields_map_back_to_FIELD_OWNERS", () => {
  // Spot-check that fieldOwner delegates to FIELD_OWNERS for static fields.
  const definition = makeDefinition();
  const ctx = { kind: "definition-only", definition };

  // displayName, avatarUrl, runtime, envVars, namePool are always D.
  for (const field of [
    "displayName",
    "avatarUrl",
    "runtime",
    "envVars",
    "namePool",
  ]) {
    assert.equal(
      fieldOwner(field, ctx),
      "definition",
      `${field} must be D-owned`,
    );
    assert.equal(
      FIELD_OWNERS[field],
      "definition",
      `FIELD_OWNERS[${field}] must be definition`,
    );
  }
});

test("test_emitAgentFormDiff_routes_respondTo_change_to_agentInput_via_fieldOwner", () => {
  // Verify that emitAgentFormDiff uses fieldOwner to route respondTo for linked context.
  const definition = makeDefinition({ respondTo: "anyone" });
  const instance = makeInstance({ respondTo: "anyone" });
  const ctx = { kind: "instance-with-definition", definition, instance };

  const seed = seedAgentFormModel(ctx);
  const changed = { ...seed, respondTo: "owner-only" };
  const emit = emitAgentFormDiff(seed, changed, ctx);

  // respondTo change must land in agentInput (I-owned), NOT personaInput.
  assert.ok(emit.agentInput, "agentInput must be present for respondTo change");
  assert.equal(
    emit.agentInput.respondTo,
    "owner-only",
    "changed respondTo must appear in agentInput",
  );
  assert.equal(
    emit.personaInput,
    null,
    "personaInput must be null — respondTo is I-owned in linked context",
  );
});

test("test_emitAgentFormDiff_routes_respondTo_change_to_personaInput_via_fieldOwner_in_definition_only", () => {
  // In definition-only context, fieldOwner returns "definition" for respondTo.
  const definition = makeDefinition({ respondTo: "anyone" });
  const ctx = { kind: "definition-only", definition };

  const seed = seedAgentFormModel(ctx);
  const changed = { ...seed, respondTo: "owner-only" };
  const emit = emitAgentFormDiff(seed, changed, ctx);

  // respondTo change must land in personaInput (D-owned in definition-only context).
  assert.ok(
    emit.personaInput,
    "personaInput must be present for respondTo change in definition-only",
  );
  assert.equal(
    emit.personaInput.behavior?.respondTo,
    "owner-only",
    "changed respondTo must appear in personaInput.behavior",
  );
  assert.equal(
    emit.agentInput,
    null,
    "agentInput must be null — no instance in definition-only context",
  );
});

// ── D-section tuning/API-key env writes route to the DEFINITION layer ─────────
//
// The restored D-section Advanced knobs (numeric tuning, effort, provider API
// key) all mutate the same `envVars` model field — the definition env layer.
// Paul's parity mandate: definition fields → personaInput, instance fields →
// agentInput. These probe that a D-env edit lands in personaInput.envVars and
// never leaks into the instance overlay (agentInput.envVars), in both contexts
// where the D-section renders (linked and definition-only).

test("test_definition_tuning_env_edit_routes_to_personaInput_not_instance_overlay", () => {
  // Linked agent: the D-section edits the definition env, while the instance
  // keeps its own overlay. A tuning-knob write must reach personaInput.envVars
  // and leave the instance overlay (and agentInput) untouched.
  const definition = makeDefinition({ runtime: "buzz-agent", envVars: {} });
  const instance = makeInstance({ envVars: { PER_INSTANCE: "keep" } });
  const ctx = { kind: "instance-with-definition", definition, instance };
  const seed = seedAgentFormModel(ctx);

  // User sets a numeric tuning knob (writes to the D-env layer, `envVars`).
  const next = {
    ...seed,
    envVars: { ...seed.envVars, GOOSE_MAX_OUTPUT_TOKENS: "8192" },
  };
  const emit = emitAgentFormDiff(seed, next, ctx);

  assert.ok(emit.personaInput, "a D-env tuning edit must emit a personaInput");
  assert.equal(
    emit.personaInput.envVars?.GOOSE_MAX_OUTPUT_TOKENS,
    "8192",
    "the tuning value must land in personaInput.envVars (definition layer)",
  );
  assert.equal(
    emit.agentInput,
    null,
    "the instance overlay is untouched, so no agentInput is emitted",
  );
});

test("test_definition_api_key_env_edit_routes_to_personaInput_in_definition_only", () => {
  // Definition-only edit: the restored provider API-key field writes the secret
  // into the definition env. It must travel as personaInput.envVars — there is
  // no instance layer to leak into.
  const definition = makeDefinition({
    runtime: "buzz-agent",
    provider: "anthropic",
    envVars: {},
  });
  const ctx = { kind: "definition-only", definition };
  const seed = seedAgentFormModel(ctx);

  // User types the provider API key (writes ANTHROPIC_API_KEY into `envVars`).
  const next = {
    ...seed,
    envVars: { ...seed.envVars, ANTHROPIC_API_KEY: "sk-live-xyz" },
  };
  const emit = emitAgentFormDiff(seed, next, ctx);

  assert.ok(
    emit.personaInput,
    "a definition-only API-key edit must emit a personaInput",
  );
  assert.equal(
    emit.personaInput.envVars?.ANTHROPIC_API_KEY,
    "sk-live-xyz",
    "the API key must land in personaInput.envVars (definition layer)",
  );
  assert.equal(
    emit.agentInput,
    null,
    "definition-only context has no instance, so agentInput must be null",
  );
});
