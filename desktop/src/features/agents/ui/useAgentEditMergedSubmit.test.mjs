import assert from "node:assert/strict";
import test from "node:test";

import { buildNextAgentFormModel } from "./useAgentEditMergedSubmit.ts";
import { seedAgentFormModel, emitAgentFormDiff } from "./agentFormModel.ts";

// ── Shared fixtures ────────────────────────────────────────────────────────────

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
    respondTo: "anyone",
    respondToAllowlist: [],
    parallelism: 4,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

/**
 * Minimal AgentEditSubmitState that mirrors a definition-only edit form. Only
 * the fields buildNextAgentFormModel reads are populated; `parallelism` is the
 * live text-input value and `parsedParallelism` is `parseInt` of it.
 */
function makeDefinitionOnlyState(definition, overrides = {}) {
  const ctx = { kind: "definition-only", definition };
  return {
    ctx,
    displayName: definition.displayName,
    avatarUrl: definition.avatarUrl,
    systemPrompt: definition.systemPrompt,
    namePoolText: "",
    model: definition.model ?? "",
    provider: definition.provider ?? "",
    respondTo: definition.respondTo,
    respondToAllowlist: definition.respondToAllowlist,
    parallelism:
      definition.parallelism != null ? String(definition.parallelism) : "",
    parsedParallelism:
      definition.parallelism != null ? definition.parallelism : Number.NaN,
    envVars: definition.envVars,
    instanceEnvVars: undefined,
    instanceName: "",
    autoRestartOnConfigChange: undefined,
    startOnAppLaunch: undefined,
    definitionRuntimeId: definition.runtime ?? "custom",
    autoSeededDefinitionRuntimeRef: { current: null },
    selectedRuntimeId: "",
    inheritHarness: undefined,
    agentCommand: "",
    agentArgs: "",
    acpCommand: "",
    showInst: false,
    defReadOnly: false,
    inheritedSubmissionProvider: null,
    runtimes: [],
    ...overrides,
  };
}

// ── Finding 2: clearing definition parallelism reaches the wire ───────────────
//
// A definition-only form presents parallelism as optional. Blanking a stored
// value must dirty the D-field and emit a behavior group that clears it — the
// backend's PersonaBehaviorRequest is a full-replacement unit and clears an
// OMITTED member. Previously a blank input restored the seed, so the clear was
// silently discarded (no D-change, Save left the stored value in place).

test("test_definition_only_blank_parallelism_emits_clearing_behavior_group", () => {
  const definition = makeDefinition({ parallelism: 4 });
  const ctx = { kind: "definition-only", definition };
  const seed = seedAgentFormModel(ctx);
  assert.equal(seed.parallelism, 4, "seed carries the stored parallelism");

  // User blanks the parallelism input.
  const next = buildNextAgentFormModel(
    seed,
    makeDefinitionOnlyState(definition, {
      parallelism: "",
      parsedParallelism: Number.NaN,
    }),
  );
  assert.equal(
    next.parallelism,
    null,
    "blank parallelism must become an explicit clear (null), not restore the seed",
  );

  const emit = emitAgentFormDiff(seed, next, ctx);
  assert.notEqual(
    emit.personaInput,
    null,
    "clearing parallelism must dirty the D-field and emit a personaInput",
  );
  // The clear travels as an OMITTED parallelism member in the behavior group —
  // that is exactly how the backend clears it (apply_persona_behavior sets
  // record.parallelism = None when the member is absent).
  const wire = JSON.parse(JSON.stringify(emit.personaInput.behavior ?? {}));
  assert.equal(
    "parallelism" in wire,
    false,
    "the emitted behavior group omits parallelism, clearing it on the wire",
  );
  assert.equal(
    wire.respondTo,
    "anyone",
    "the unchanged sibling (respondTo) is carried in the full-replacement group",
  );
});

test("test_definition_only_unchanged_blank_parallelism_emits_no_diff", () => {
  // A definition that already has no parallelism, left blank, must not emit a
  // phantom write.
  const definition = makeDefinition({ parallelism: null });
  const ctx = { kind: "definition-only", definition };
  const seed = seedAgentFormModel(ctx);

  const next = buildNextAgentFormModel(
    seed,
    makeDefinitionOnlyState(definition, {
      parallelism: "",
      parsedParallelism: Number.NaN,
    }),
  );
  assert.equal(next.parallelism, null, "blank stays null");

  const emit = emitAgentFormDiff(seed, next, ctx);
  assert.equal(
    emit.personaInput,
    null,
    "null → blank is a no-op and must not emit a personaInput",
  );
});

test("test_definition_only_valid_parallelism_is_carried_through", () => {
  // A valid positive value edit still emits it as the group value.
  const definition = makeDefinition({ parallelism: 4 });
  const ctx = { kind: "definition-only", definition };
  const seed = seedAgentFormModel(ctx);

  const next = buildNextAgentFormModel(
    seed,
    makeDefinitionOnlyState(definition, {
      parallelism: "8",
      parsedParallelism: 8,
    }),
  );
  assert.equal(next.parallelism, 8, "valid value is carried through");

  const emit = emitAgentFormDiff(seed, next, ctx);
  assert.notEqual(
    emit.personaInput,
    null,
    "a parallelism change emits personaInput",
  );
  assert.equal(
    emit.personaInput.behavior?.parallelism,
    8,
    "the behavior group carries the new parallelism value",
  );
});
