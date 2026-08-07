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

test('shared compute encodes Auto as "auto", not blank', () => {
  // Blank means *inherit the global model*, so on shared compute a blank Auto
  // resolves to whatever unrelated model the global config names -- a Claude id
  // sent to a local mesh node -- and it fails the raw-pair submit gate in
  // `agentAiConfigurationModeSatisfied`, which is what greyed out Create.
  //
  // "auto" is also a trigger token, not a final choice: buzz-agent rewrites
  // exactly "auto" to the virtual `mesh` model when the catalog offers enough
  // models for Mixture-of-Agents. Any concrete id defeats that translation.
  const auto = selectionOnModelDropdownChange(
    { ...base, provider: "relay-mesh", model: "some-physical-model" },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
      isRelayMesh: true,
    },
  );
  assert.equal(auto.model, "auto");
  assert.equal(auto.isCustomModelEditing, false);
});

test("a concrete shared-compute model is still honoured verbatim", () => {
  // Only Auto is special. An explicit pick must reach mesh-llm unchanged.
  const concrete = selectionOnModelDropdownChange(
    { ...base, provider: "relay-mesh", model: "auto" },
    {
      nextValue: "unsloth/gemma-4-26B",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
      isRelayMesh: true,
    },
  );
  assert.equal(concrete.model, "unsloth/gemma-4-26B");
});

test("ordinary providers keep blank for Auto", () => {
  // Blank is the generic "inherit the global default" encoding everywhere else,
  // and must not be migrated to "auto".
  const auto = selectionOnModelDropdownChange(
    { ...base, provider: "anthropic", model: "claude-opus-5" },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
      isRelayMesh: false,
    },
  );
  assert.equal(auto.model, "");
  // Omitting the flag entirely behaves the same, so existing callers are safe.
  const omitted = selectionOnModelDropdownChange(
    { ...base, provider: "anthropic", model: "claude-opus-5" },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
    },
  );
  assert.equal(omitted.model, "");
});

test("shared compute is inferred from the selection when the caller omits it", () => {
  // The Edit dialog cannot pass the flag (its file is at the size ceiling), so
  // the selection's own provider has to be enough for the common case where the
  // agent names relay-mesh itself.
  const auto = selectionOnModelDropdownChange(
    { ...base, provider: "relay-mesh", model: "some-physical-model" },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
    },
  );
  assert.equal(auto.model, "auto");
});

test("an explicit flag wins over the selection's own provider", () => {
  // Inheritance case: the picker can run off an effective provider the selection
  // does not name, so a caller that knows better must be able to say so.
  const inherited = selectionOnModelDropdownChange(
    { ...base, provider: "", model: "x" },
    {
      nextValue: "__auto_model__",
      clearKnownModelOnCustomEntry: false,
      isModelCustom: false,
      isRelayMesh: true,
    },
  );
  assert.equal(inherited.model, "auto");
});
