import assert from "node:assert/strict";
import test from "node:test";

import { getProviderApiKeyFieldState } from "./providerApiKeyFieldState.ts";

test("providerApiKeyFieldState_missingAnthropicKey_isTopLevelRequired", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.secretEnvVar, "ANTHROPIC_API_KEY");
  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
  assert.deepEqual(state.advancedRequiredEnvKeys, []);
});

test("providerApiKeyFieldState_globalCredential_isInheritedWithoutDuplicateAdvancedRow", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    provider: "anthropic",
    requiredEnvKeys: [],
  });

  assert.equal(state.isRequired, false);
  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from global config");
  assert.deepEqual(state.advancedRequiredEnvKeys, []);
});

test("providerApiKeyFieldState_personaCredential_isInheritedForInstanceTransition", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "persona-secret" },
    envVars: {},
    globalEnvVars: {},
    personaSatisfied: true,
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, false);
  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from agent profile");
});

test("providerApiKeyFieldState_explicitLocalEmptyShadowsGlobalAndBuild", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: ["ANTHROPIC_API_KEY"],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "" },
    envVars: { ANTHROPIC_API_KEY: "" },
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
  assert.equal(state.inheritedLabel, "");
});

test("providerApiKeyFieldState_openaiUsesCompatCredentialKey", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "openai",
    requiredEnvKeys: ["OPENAI_COMPAT_API_KEY"],
  });

  assert.equal(state.secretEnvVar, "OPENAI_COMPAT_API_KEY");
  assert.equal(state.isRequired, true);
});

test("providerApiKeyFieldState_explicitLocalEmptyShadowsRuntimeConfig", () => {
  const state = getProviderApiKeyFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: { ANTHROPIC_API_KEY: "" },
    envVars: { ANTHROPIC_API_KEY: "" },
    fileSatisfiedEnvKeys: ["ANTHROPIC_API_KEY"],
    globalEnvVars: {},
    provider: "anthropic",
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
  });

  assert.equal(state.isRequired, true);
  assert.equal(state.isInherited, false);
});
