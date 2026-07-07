import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveInheritedRuntimeSubmission,
  shouldClearModelForRuntimeChange,
} from "./personaRuntimeModel.ts";

test("shouldClearModelForRuntimeChange preserves model for first runtime selection", () => {
  assert.equal(shouldClearModelForRuntimeChange("", "goose"), false);
});

test("shouldClearModelForRuntimeChange clears model when switching runtimes", () => {
  assert.equal(shouldClearModelForRuntimeChange("goose", "claude"), true);
});

test("shouldClearModelForRuntimeChange clears model when runtime is removed", () => {
  assert.equal(shouldClearModelForRuntimeChange("goose", ""), true);
});

test("shouldClearModelForRuntimeChange keeps model for unchanged runtime", () => {
  assert.equal(shouldClearModelForRuntimeChange("goose", "goose"), false);
});

test("resolveInheritedRuntimeSubmission passes through local edit state when not inheriting", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: false,
    agentWasHarnessPinned: false,
    provider: "databricks",
    personaProvider: "anthropic",
    envVars: { FOO: "bar" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "databricks");
  assert.deepEqual(result.envVars, { FOO: "bar" });
});

test("resolveInheritedRuntimeSubmission normalizes an empty local provider to null when not inheriting", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: false,
    agentWasHarnessPinned: true,
    provider: "   ",
    personaProvider: "anthropic",
    envVars: {},
    personaEnvVars: {},
  });
  assert.equal(result.provider, null);
});

test("resolveInheritedRuntimeSubmission persists the persona provider + layered env on the inherit-transition from a harness pin", () => {
  // The core fix: a previously harness-pinned agent has a cleared provider and
  // no credential locally, but on the inherit-transition the persona snapshot
  // must be persisted so the record (which spawn reads) carries the provider +
  // credential. Requires agentWasHarnessPinned to distinguish this from a
  // steady-state inherit.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "anthropic");
  assert.deepEqual(result.envVars, { ANTHROPIC_API_KEY: "sk-persona" });
});

test("resolveInheritedRuntimeSubmission layers the agent's own env over the persona's on the inherit-transition", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    envVars: { ANTHROPIC_API_KEY: "sk-agent", EXTRA: "1" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  // Agent layer wins on key collision, mirroring spawn-time layering.
  assert.deepEqual(result.envVars, {
    ANTHROPIC_API_KEY: "sk-agent",
    EXTRA: "1",
  });
});

test("resolveInheritedRuntimeSubmission preserves a user-edited provider + env while inheriting", () => {
  // Regression: an already-inheriting agent (e.g. an Anthropic persona) that
  // the user re-points to Databricks with its own DATABRICKS_HOST must persist
  // that deliberate edit verbatim — NOT get overwritten with the persona's
  // provider/env. The provider field is user-editable even while inheriting.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: false,
    provider: "databricks",
    personaProvider: "anthropic",
    envVars: { DATABRICKS_HOST: "https://dbc-x.cloud.databricks.com" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "databricks");
  assert.deepEqual(result.envVars, {
    DATABRICKS_HOST: "https://dbc-x.cloud.databricks.com",
  });
});

test("resolveInheritedRuntimeSubmission clears an already-inheriting agent's provider override when the user picks Default", () => {
  // Regression: an already-inheriting agent had a saved provider override
  // (databricks). The user picks the "Default" option → empty local provider.
  // Because the agent was NOT harness-pinned at open, this is a deliberate
  // clear, not the inherit-transition — persist null (runtime default), do NOT
  // resurrect the persona provider.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: false,
    provider: "",
    personaProvider: "anthropic",
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, null);
  assert.deepEqual(result.envVars, {});
});

test("resolveInheritedRuntimeSubmission normalizes a whitespace-only local provider on the inherit-transition (unset persona)", () => {
  // The inherit-transition branch (was harness-pinned, now inheriting, empty
  // local provider); an unset persona provider normalizes to null.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "   ",
    personaProvider: "",
    envVars: {},
    personaEnvVars: {},
  });
  assert.equal(result.provider, null);
  assert.deepEqual(result.envVars, {});
});
