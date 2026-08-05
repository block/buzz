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

test("resolveInheritedRuntimeSubmission clears an already-inheriting agent's persona-backed provider and model to agent defaults", () => {
  // Regression: an already-inheriting agent is linked to a persona with a
  // provider and model. The user picks "Use agent defaults" for both fields.
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

// ── Linked Goose instance: effort persistence and inheritance ─────────────────
//
// When a linked Goose instance selects Inherit for effort, the record must NOT
// acquire the persona's effort key — inheritance happens at spawn time. The
// `excludePersonaEnvKeys` parameter strips harness-native effort keys from the
// persona layer on the inherit-transition so the record stays override-free.

const GOOSE_EFFORT_KEY = "GOOSE_THINKING_EFFORT";
const BUZZ_EFFORT_KEY = "BUZZ_AGENT_THINKING_EFFORT";
// Keys stripped from persona layer for a Goose linked instance.
const GOOSE_EXCLUDE_EFFORT_KEYS = [GOOSE_EFFORT_KEY, BUZZ_EFFORT_KEY];

test("linked_goose_instance_inherit_does_not_materialize_persona_native_effort", () => {
  // Persona has GOOSE_THINKING_EFFORT=high. User selects Inherit → local envVars
  // has no effort key. Without excludePersonaEnvKeys the inherited-transition
  // would persist GOOSE_THINKING_EFFORT=high into the record.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "",
    model: "",
    personaModel: null,
    envVars: {}, // user selected Inherit — no local effort override
    personaEnvVars: { [GOOSE_EFFORT_KEY]: "high" },
    excludePersonaEnvKeys: GOOSE_EXCLUDE_EFFORT_KEYS,
  });
  assert.equal(
    Object.hasOwn(result.envVars, GOOSE_EFFORT_KEY),
    false,
    "persona native effort key must NOT materialize into record on Inherit",
  );
});

test("linked_goose_instance_inherit_does_not_materialize_persona_legacy_effort", () => {
  // Same as above but persona has the legacy BUZZ_AGENT_THINKING_EFFORT key
  // (pre-migration persona). Must also be excluded.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "",
    model: "",
    personaModel: null,
    envVars: {},
    personaEnvVars: { [BUZZ_EFFORT_KEY]: "high" },
    excludePersonaEnvKeys: GOOSE_EXCLUDE_EFFORT_KEYS,
  });
  assert.equal(
    Object.hasOwn(result.envVars, BUZZ_EFFORT_KEY),
    false,
    "persona legacy effort key must NOT materialize into record on Inherit",
  );
});

test("linked_goose_instance_concrete_effort_override_wins_over_persona", () => {
  // When the user has a local effort override, it must win over persona — the
  // exclude list should not strip it (local wins naturally via envVars layering).
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "",
    model: "",
    personaModel: null,
    envVars: { [GOOSE_EFFORT_KEY]: "medium" }, // user selected a concrete value
    personaEnvVars: { [GOOSE_EFFORT_KEY]: "high" },
    excludePersonaEnvKeys: GOOSE_EXCLUDE_EFFORT_KEYS,
  });
  assert.equal(
    result.envVars[GOOSE_EFFORT_KEY],
    "medium",
    "local effort override wins over persona",
  );
});

test("linked_goose_instance_non_effort_persona_env_still_merged", () => {
  // excludePersonaEnvKeys must not affect non-effort keys — credentials and
  // other persona env vars must still be included in the inherit-transition.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "",
    model: "",
    personaModel: null,
    envVars: {},
    personaEnvVars: {
      [GOOSE_EFFORT_KEY]: "high",
      SOME_OTHER_KEY: "persona_value",
    },
    excludePersonaEnvKeys: GOOSE_EXCLUDE_EFFORT_KEYS,
  });
  assert.equal(
    Object.hasOwn(result.envVars, GOOSE_EFFORT_KEY),
    false,
    "effort key excluded from persona",
  );
  assert.equal(
    result.envVars.SOME_OTHER_KEY,
    "persona_value",
    "non-effort persona env vars still merged",
  );
});

test("linked_goose_instance_submit_payload_contains_no_inherited_effort_on_inherit", () => {
  // Full scenario: harness-pinned Goose instance. Persona has GOOSE_THINKING_EFFORT=max.
  // On inherit-transition (user selects Inherit, clears local effort), the submit
  // payload must have neither native nor legacy effort key.
  const result = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: "anthropic",
    model: "",
    personaModel: null,
    envVars: { ANTHROPIC_API_KEY: "sk-agent" }, // credential is local, no effort
    personaEnvVars: {
      ANTHROPIC_API_KEY: "sk-persona",
      [GOOSE_EFFORT_KEY]: "max",
    },
    excludePersonaEnvKeys: GOOSE_EXCLUDE_EFFORT_KEYS,
  });
  assert.equal(
    Object.hasOwn(result.envVars, GOOSE_EFFORT_KEY),
    false,
    "no native effort key in submit payload on Inherit",
  );
  assert.equal(
    Object.hasOwn(result.envVars, BUZZ_EFFORT_KEY),
    false,
    "no legacy effort key in submit payload on Inherit",
  );
  // Credential still present (agent's own layer wins over persona).
  assert.equal(
    result.envVars.ANTHROPIC_API_KEY,
    "sk-agent",
    "agent-local credential preserved",
  );
});
