import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveInheritedRuntimeSubmission,
  resolveRuntimeProviderCapability,
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
    model: "",
    personaModel: "claude-sonnet",
    envVars: { FOO: "bar" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "databricks");
  assert.deepEqual(result.envVars, { FOO: "bar" });
  // Not inheriting → persona model is never substituted; empty local → null.
  assert.equal(result.model, null);
});

test("resolveInheritedRuntimeSubmission normalizes an empty local provider to null when not inheriting", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: false,
    agentWasHarnessPinned: true,
    provider: "   ",
    personaProvider: "anthropic",
    model: "",
    personaModel: "claude-sonnet",
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
    model: "",
    personaModel: "claude-sonnet",
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "anthropic");
  assert.deepEqual(result.envVars, { ANTHROPIC_API_KEY: "sk-persona" });
  // Empty local model on the transition inherits the persona model so a
  // provider-backed runtime isn't saved model-less (readiness requires one).
  assert.equal(result.model, "claude-sonnet");
});

test("resolveInheritedRuntimeSubmission layers the agent's own env over the persona's on the inherit-transition", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    model: "",
    personaModel: "claude-sonnet",
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
    model: "",
    personaModel: "claude-sonnet",
    envVars: { DATABRICKS_HOST: "https://dbc-x.cloud.databricks.com" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, "databricks");
  assert.deepEqual(result.envVars, {
    DATABRICKS_HOST: "https://dbc-x.cloud.databricks.com",
  });
  // Not the transition branch → persona model is NOT substituted.
  assert.equal(result.model, null);
});

test("resolveInheritedRuntimeSubmission clears an already-inheriting agent's persona-backed provider and model to AI defaults", () => {
  // Regression: an already-inheriting agent is linked to a persona with a
  // provider and model. The user picks "Use AI defaults" for both fields.
  // Because the agent was NOT harness-pinned at open, these empty local values
  // are deliberate clears, not an inherit-transition — persist null for both
  // rather than resurrecting the persona values.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: false,
    provider: "",
    personaProvider: "anthropic",
    model: "",
    personaModel: "claude-sonnet",
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.provider, null);
  assert.equal(result.model, null);
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
    model: "",
    personaModel: null,
    envVars: {},
    personaEnvVars: {},
  });
  assert.equal(result.provider, null);
  assert.deepEqual(result.envVars, {});
});

test("resolveInheritedRuntimeSubmission keeps a deliberate local model on the inherit-transition", () => {
  // A non-empty local model is an explicit pick and wins over the persona
  // model even on the transition branch.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    model: "claude-opus",
    personaModel: "claude-sonnet",
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.model, "claude-opus");
});

test("resolveInheritedRuntimeSubmission yields a null model on the inherit-transition when the persona has none", () => {
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    model: "",
    personaModel: null,
    envVars: {},
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-persona" },
  });
  assert.equal(result.model, null);
});

test("resolveRuntimeProviderCapability classifies provider-capable runtimes as capable", () => {
  assert.equal(resolveRuntimeProviderCapability("buzz-agent", true), "capable");
  assert.equal(resolveRuntimeProviderCapability("goose", true), "capable");
});

test("resolveRuntimeProviderCapability classifies known CLI-login runtimes as locked before the catalog loads", () => {
  // The core fix: a not-yet-loaded catalog must not force these to "unknown".
  assert.equal(resolveRuntimeProviderCapability("claude", false), "locked");
  assert.equal(resolveRuntimeProviderCapability("codex", false), "locked");
  assert.equal(resolveRuntimeProviderCapability(" claude ", false), "locked");
});

test("resolveRuntimeProviderCapability leaves genuinely unknown/custom runtimes as unknown", () => {
  // Preserves the tri-state's "omit rather than destructively write" behavior
  // for ids we can't statically classify (custom command, empty, unknown).
  assert.equal(resolveRuntimeProviderCapability("custom", false), "unknown");
  assert.equal(resolveRuntimeProviderCapability("", false), "unknown");
  assert.equal(
    resolveRuntimeProviderCapability("some-vendor-cli", false),
    "unknown",
  );
});

// ── F2 regression: inherit-transition merges persona env into discovery snapshot ─
//
// AgentInstanceEditDialog's envVarsForDiscovery is now:
//   { ...globalConfig.env_vars, ...inheritedSubmission.envVars }
// On an inherit-transition (was harness-pinned, now inheriting with empty local
// provider), resolveInheritedRuntimeSubmission returns
//   envVars = { ...personaEnvVars, ...agentLocalEnvVars }
// This test pins that persona env vars are included in the submission snapshot,
// proving that a credential held only in the persona's env satisfies discovery
// after the fix (not discoverable without F2's snapshot alignment).

test("resolveInheritedRuntimeSubmission inherit-transition includes persona credential in envVars for discovery", () => {
  // Scenario: agent was harness-pinned (agentCommandOverride != null).
  // User switches to inherit. Persona has ANTHROPIC_API_KEY; agent-local does not.
  // The resolved envVars must include ANTHROPIC_API_KEY so discovery can use it.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "", // empty → inherit-transition path
    personaProvider: "anthropic",
    model: "",
    personaModel: "claude-opus-4-5",
    envVars: {}, // no agent-local credential
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-from-persona" },
  });

  assert.equal(
    result.envVars.ANTHROPIC_API_KEY,
    "sk-from-persona",
    "persona ANTHROPIC_API_KEY must be present in resolved envVars on inherit-transition",
  );
  // envVarsForDiscovery = { ...global, ...result.envVars } would then include
  // ANTHROPIC_API_KEY, preventing the misleading 'Enter an API key' hint.
});

test("resolveInheritedRuntimeSubmission agent-local env overrides persona credential on inherit-transition", () => {
  // Agent-local key wins over persona key — consistent with the spawn-path
  // precedence used everywhere else.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    model: "",
    personaModel: null,
    envVars: { ANTHROPIC_API_KEY: "sk-from-agent" },
    personaEnvVars: { ANTHROPIC_API_KEY: "sk-from-persona" },
  });

  assert.equal(
    result.envVars.ANTHROPIC_API_KEY,
    "sk-from-agent",
    "agent-local credential must override persona credential on inherit-transition",
  );
});
