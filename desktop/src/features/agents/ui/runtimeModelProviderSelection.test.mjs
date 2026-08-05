import assert from "node:assert/strict";
import test from "node:test";

import {
  selectionOnModelDropdownChange,
  selectionOnProviderDropdownChange,
  selectionOnRuntimeChange,
} from "./runtimeModelProviderSelection.ts";

const base = {
  provider: "",
  model: "",
  isCustomProviderEditing: false,
  isCustomModelEditing: false,
  envVars: {},
};

// --- selectionOnRuntimeChange ---

test("runtime change to a provider-locked runtime, full reset (Persona/Edit): clears provider, custom flags, and managed API key", () => {
  const next = selectionOnRuntimeChange(
    {
      ...base,
      provider: "anthropic",
      model: "claude-4",
      isCustomProviderEditing: true,
      isCustomModelEditing: true,
      envVars: { ANTHROPIC_API_KEY: "sk-1", KEEP: "x" },
    },
    {
      previousRuntime: "buzz-agent",
      nextRuntime: "claude",
      nextRuntimeCanChooseProvider: false,
      lockedRuntimeReset: "full",
    },
  );
  assert.equal(next.provider, "");
  assert.equal(next.isCustomProviderEditing, false);
  assert.equal(next.isCustomModelEditing, false);
  assert.deepEqual(next.envVars, { KEEP: "x" });
});

test("runtime change to a provider-locked runtime, provider-only reset (Create): keeps env vars and custom-model flag", () => {
  const next = selectionOnRuntimeChange(
    {
      ...base,
      provider: "anthropic",
      envVars: { ANTHROPIC_API_KEY: "sk-1" },
      isCustomModelEditing: true,
      model: "my-custom",
    },
    {
      previousRuntime: "buzz-agent",
      nextRuntime: "claude",
      nextRuntimeCanChooseProvider: false,
      lockedRuntimeReset: "provider-only",
    },
  );
  assert.equal(next.provider, "");
  assert.equal(next.isCustomProviderEditing, false);
  assert.deepEqual(next.envVars, { ANTHROPIC_API_KEY: "sk-1" });
});

test("runtime change between provider-selection runtimes keeps provider state", () => {
  const current = {
    ...base,
    provider: "anthropic",
    envVars: { ANTHROPIC_API_KEY: "sk-1" },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "goose",
    nextRuntime: "buzz-agent",
    nextRuntimeCanChooseProvider: true,
    lockedRuntimeReset: "full",
  });
  assert.equal(next.provider, "anthropic");
  assert.deepEqual(next.envVars, { ANTHROPIC_API_KEY: "sk-1" });
});

// --- selectionOnProviderDropdownChange ---

test("provider switch clears the previous managed API key and sets the provider", () => {
  const next = selectionOnProviderDropdownChange(
    {
      ...base,
      provider: "anthropic",
      envVars: { ANTHROPIC_API_KEY: "sk-1", KEEP: "x" },
    },
    {
      runtime: "buzz-agent",
      nextValue: "openai",
      clearModelWhenApiKeyMissing: false,
    },
  );
  assert.equal(next.provider, "openai");
  assert.equal(next.isCustomProviderEditing, false);
  assert.deepEqual(next.envVars, { KEEP: "x" });
});

test("custom-provider entry clears the managed key and enters custom editing", () => {
  const next = selectionOnProviderDropdownChange(
    {
      ...base,
      provider: "anthropic",
      envVars: { ANTHROPIC_API_KEY: "sk-1" },
    },
    {
      runtime: "buzz-agent",
      nextValue: "__custom_provider__",
      clearModelWhenApiKeyMissing: false,
    },
  );
  assert.equal(next.isCustomProviderEditing, true);
  assert.equal(next.provider, "");
  assert.deepEqual(next.envVars, {});
});

test("auto-provider selection maps to empty provider", () => {
  const next = selectionOnProviderDropdownChange(
    { ...base, provider: "anthropic", envVars: { ANTHROPIC_API_KEY: "sk-1" } },
    {
      runtime: "buzz-agent",
      nextValue: "__auto_provider__",
      clearModelWhenApiKeyMissing: false,
    },
  );
  assert.equal(next.provider, "");
  assert.deepEqual(next.envVars, {});
});

test("Persona mode clears the model when the new provider's API key is missing", () => {
  const next = selectionOnProviderDropdownChange(
    { ...base, model: "claude-4", provider: "" },
    {
      runtime: "buzz-agent",
      nextValue: "anthropic",
      clearModelWhenApiKeyMissing: true,
    },
  );
  assert.equal(next.model, "");
});

test("Create/Edit mode keeps the model when the new provider's API key is missing", () => {
  // claude-4 is scope-agnostic here: shouldClearKnownModelForSelectionScope
  // only clears known models for the selection scope, and a custom string
  // stays put — mirroring the dialogs' behavior without the persona flag.
  const next = selectionOnProviderDropdownChange(
    { ...base, model: "my-custom-model", provider: "" },
    {
      runtime: "buzz-agent",
      nextValue: "anthropic",
      clearModelWhenApiKeyMissing: false,
    },
  );
  assert.equal(next.model, "my-custom-model");
});

test("custom-model editing suppresses the model-scope clear on provider switch", () => {
  const next = selectionOnProviderDropdownChange(
    { ...base, model: "anything", isCustomModelEditing: true },
    {
      runtime: "buzz-agent",
      nextValue: "openai",
      clearModelWhenApiKeyMissing: false,
    },
  );
  assert.equal(next.model, "anything");
  assert.equal(next.isCustomModelEditing, true);
});

// --- selectionOnModelDropdownChange ---

test("custom-model entry with clear (Persona) drops a known model", () => {
  const next = selectionOnModelDropdownChange(
    { ...base, model: "known-model" },
    {
      nextValue: "__custom_model__",
      clearKnownModelOnCustomEntry: true,
      isModelCustom: false,
    },
  );
  assert.equal(next.isCustomModelEditing, true);
  assert.equal(next.model, "");
});

test("custom-model entry keeps an already-custom model (Persona) and any model (Edit)", () => {
  const personaCustom = selectionOnModelDropdownChange(
    { ...base, model: "already-custom" },
    {
      nextValue: "__custom_model__",
      clearKnownModelOnCustomEntry: true,
      isModelCustom: true,
    },
  );
  assert.equal(personaCustom.model, "already-custom");

  const edit = selectionOnModelDropdownChange(
    { ...base, model: "known-model" },
    {
      nextValue: "__custom_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
    },
  );
  assert.equal(edit.model, "known-model");
  assert.equal(edit.isCustomModelEditing, true);
});

test("auto-model selection clears the model; concrete selection sets it", () => {
  const auto = selectionOnModelDropdownChange(
    { ...base, model: "old", isCustomModelEditing: true },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
    },
  );
  assert.equal(auto.model, "");
  assert.equal(auto.isCustomModelEditing, false);

  const concrete = selectionOnModelDropdownChange(
    { ...base, model: "" },
    {
      nextValue: "gpt-5",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
    },
  );
  assert.equal(concrete.model, "gpt-5");
});

// ── selectionOnRuntimeChange: effort-key cleanup ──────────────────────────
//
// At record/persona scope, a runtime change must clean up the previous
// runtime's native effort key AND the legacy key (BUZZ_AGENT_THINKING_EFFORT).
// The next runtime's native key is also cleared (stale leftover removal).
// Unrelated env keys are preserved.

test("selectionOnRuntimeChange_goose_to_buzz_clears_goose_native_and_legacy", () => {
  // Goose→buzz-agent: GOOSE_THINKING_EFFORT (prev native) + BUZZ_AGENT_THINKING_EFFORT
  // (legacy, also next native) are both cleared. Unrelated keys survive.
  const current = {
    ...base,
    envVars: {
      GOOSE_THINKING_EFFORT: "medium",
      BUZZ_AGENT_THINKING_EFFORT: "high",
      SOME_KEY: "value",
    },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "goose",
    nextRuntime: "buzz-agent",
    nextRuntimeCanChooseProvider: false,
    lockedRuntimeReset: "full",
    previousRuntimeNativeEffortKey: "GOOSE_THINKING_EFFORT",
    nextRuntimeNativeEffortKey: "BUZZ_AGENT_THINKING_EFFORT",
  });
  assert.equal(
    next.envVars.GOOSE_THINKING_EFFORT,
    undefined,
    "prev native effort key must be cleared",
  );
  assert.equal(
    next.envVars.BUZZ_AGENT_THINKING_EFFORT,
    undefined,
    "legacy effort key must be cleared",
  );
  assert.equal(
    next.envVars.SOME_KEY,
    "value",
    "unrelated env key must survive",
  );
});

test("selectionOnRuntimeChange_buzz_to_goose_clears_goose_native_and_legacy", () => {
  // buzz-agent→Goose: GOOSE_THINKING_EFFORT (next native) + BUZZ_AGENT_THINKING_EFFORT
  // (legacy, also prev native) are both cleared. Unrelated keys survive.
  const current = {
    ...base,
    envVars: {
      GOOSE_THINKING_EFFORT: "low",
      BUZZ_AGENT_THINKING_EFFORT: "high",
      OTHER: "keep",
    },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "buzz-agent",
    nextRuntime: "goose",
    nextRuntimeCanChooseProvider: true,
    lockedRuntimeReset: "full",
    previousRuntimeNativeEffortKey: "BUZZ_AGENT_THINKING_EFFORT",
    nextRuntimeNativeEffortKey: "GOOSE_THINKING_EFFORT",
  });
  assert.equal(
    next.envVars.GOOSE_THINKING_EFFORT,
    undefined,
    "next native effort key must be cleared",
  );
  assert.equal(
    next.envVars.BUZZ_AGENT_THINKING_EFFORT,
    undefined,
    "legacy effort key must be cleared",
  );
  assert.equal(next.envVars.OTHER, "keep", "unrelated env key must survive");
});

test("selectionOnRuntimeChange_buzz_to_buzz_preserves_effort_keys", () => {
  // buzz-agent→buzz-agent: no effort cleanup (native === legacy key,
  // both are skipped). Effort keys survive.
  const current = {
    ...base,
    envVars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "buzz-agent",
    nextRuntime: "buzz-agent",
    nextRuntimeCanChooseProvider: false,
    lockedRuntimeReset: "full",
    previousRuntimeNativeEffortKey: "BUZZ_AGENT_THINKING_EFFORT",
    nextRuntimeNativeEffortKey: "BUZZ_AGENT_THINKING_EFFORT",
  });
  assert.equal(
    next.envVars.BUZZ_AGENT_THINKING_EFFORT,
    "high",
    "buzz-agent effort key preserved on same-runtime switch",
  );
});

test("selectionOnRuntimeChange_claude_to_goose_clears_both_native_keys_and_legacy", () => {
  // claude→goose: BOTH CLAUDE_CODE_EFFORT_LEVEL (prev native) AND
  // GOOSE_THINKING_EFFORT (next native) must be cleared, plus legacy.
  // Previously only the prev-native branch fired, leaving GOOSE_THINKING_EFFORT stale.
  const current = {
    ...base,
    envVars: {
      CLAUDE_CODE_EFFORT_LEVEL: "low",
      GOOSE_THINKING_EFFORT: "max",
      BUZZ_AGENT_THINKING_EFFORT: "high",
      UNRELATED: "keep",
    },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "claude",
    nextRuntime: "goose",
    nextRuntimeCanChooseProvider: true,
    lockedRuntimeReset: "full",
    previousRuntimeNativeEffortKey: "CLAUDE_CODE_EFFORT_LEVEL",
    nextRuntimeNativeEffortKey: "GOOSE_THINKING_EFFORT",
  });
  assert.equal(
    next.envVars.CLAUDE_CODE_EFFORT_LEVEL,
    undefined,
    "source (Claude) native key must be cleared",
  );
  assert.equal(
    next.envVars.GOOSE_THINKING_EFFORT,
    undefined,
    "destination (Goose) native key must be cleared (was stale)",
  );
  assert.equal(
    next.envVars.BUZZ_AGENT_THINKING_EFFORT,
    undefined,
    "legacy key must be cleared",
  );
  assert.equal(next.envVars.UNRELATED, "keep", "unrelated key must survive");
});

test("selectionOnRuntimeChange_goose_to_claude_clears_both_native_keys_and_legacy", () => {
  // goose→claude: BOTH GOOSE_THINKING_EFFORT (prev native) AND
  // CLAUDE_CODE_EFFORT_LEVEL (next native) must be cleared, plus legacy.
  // Previously only the prev-native branch fired, leaving CLAUDE_CODE_EFFORT_LEVEL stale.
  const current = {
    ...base,
    envVars: {
      GOOSE_THINKING_EFFORT: "high",
      CLAUDE_CODE_EFFORT_LEVEL: "low",
      BUZZ_AGENT_THINKING_EFFORT: "medium",
      UNRELATED: "keep",
    },
  };
  const next = selectionOnRuntimeChange(current, {
    previousRuntime: "goose",
    nextRuntime: "claude",
    nextRuntimeCanChooseProvider: false,
    lockedRuntimeReset: "full",
    previousRuntimeNativeEffortKey: "GOOSE_THINKING_EFFORT",
    nextRuntimeNativeEffortKey: "CLAUDE_CODE_EFFORT_LEVEL",
  });
  assert.equal(
    next.envVars.GOOSE_THINKING_EFFORT,
    undefined,
    "source (Goose) native key must be cleared",
  );
  assert.equal(
    next.envVars.CLAUDE_CODE_EFFORT_LEVEL,
    undefined,
    "destination (Claude) native key must be cleared (was stale)",
  );
  assert.equal(
    next.envVars.BUZZ_AGENT_THINKING_EFFORT,
    undefined,
    "legacy key must be cleared",
  );
  assert.equal(next.envVars.UNRELATED, "keep", "unrelated key must survive");
});
