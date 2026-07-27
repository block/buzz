import assert from "node:assert/strict";
import test from "node:test";

import { getGlobalAgentCredentialState } from "./globalAgentCredentialState.ts";

test("global defaults accept an advanced credential set in runtime config", () => {
  const state = getGlobalAgentCredentialState({
    bakedEnvKeys: [],
    envVars: {},
    provider: "databricks_v2",
    runtimeFileConfig: {
      provider: "databricks_v2",
      model: "goose-claude-4-6-opus",
      satisfiedEnvKeys: ["DATABRICKS_HOST"],
    },
    runtimeId: "goose",
  });

  assert.equal(state.advancedCredentialMissing, false);
  assert.equal(state.credentialsValid, true);
  assert.deepEqual(state.advancedRequiredEnvKeys, []);
  assert.deepEqual(state.advancedFileSatisfiedEnvKeys, ["DATABRICKS_HOST"]);
});

test("an explicit empty global value shadows the runtime config", () => {
  const state = getGlobalAgentCredentialState({
    bakedEnvKeys: [],
    envVars: { DATABRICKS_HOST: "" },
    provider: "databricks_v2",
    runtimeFileConfig: {
      provider: "databricks_v2",
      model: "goose-claude-4-6-opus",
      satisfiedEnvKeys: ["DATABRICKS_HOST"],
    },
    runtimeId: "goose",
  });

  assert.equal(state.advancedCredentialMissing, true);
  assert.equal(state.credentialsValid, false);
  assert.deepEqual(state.advancedRequiredEnvKeys, ["DATABRICKS_HOST"]);
  assert.deepEqual(state.advancedFileSatisfiedEnvKeys, []);
});

test("global defaults accept a provider key set in runtime config", () => {
  const state = getGlobalAgentCredentialState({
    bakedEnvKeys: [],
    envVars: {},
    provider: "openai",
    runtimeFileConfig: {
      provider: "openai",
      model: "gpt-5.5",
      satisfiedEnvKeys: ["OPENAI_COMPAT_API_KEY"],
    },
    runtimeId: "goose",
  });

  assert.equal(state.apiKeyFileSatisfied, true);
  assert.equal(state.apiKeyInherited, true);
  assert.equal(state.credentialsValid, true);
});
