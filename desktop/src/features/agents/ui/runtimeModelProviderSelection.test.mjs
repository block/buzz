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
