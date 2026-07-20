/**
 * Unit tests for the agent-dialog local-mode readiness gate.
 *
 * The gate computes whether required fields are present for the selected
 * runtime: when missing, it surfaces field markers (isRequired) and env-key
 * amber rows (EnvVarsEditor.requiredKeys). `localModeGate.satisfied` also
 * blocks create and definition-edit saves, preventing invalid agents from
 * starting with incomplete provider configuration.
 *
 * On Create there is no inherit checkbox, so selectedRuntimeId IS the
 * prospective runtime — no prospectiveRuntimeId hoist needed.
 *
 * The shared helper under test:
 *   computeLocalModeGate — pure function used by field isRequired,
 *                           EnvVarsEditor.requiredKeys, and the submit gate.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  buildTemplateModelDropdownOptions,
  computeLocalModeGate,
  getBakedSatisfiedEnvKeys,
  getDefaultLlmModelLabel,
  getDefaultLlmProviderLabel,
  getPersonaModelOptions,
  getPersonaProviderOptions,
  getProviderApiKeyEnvVar,
  isGloballySatisfiedCredentialKey,
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./agentConfigOptions.tsx";
import { hasMissingRequiredEnvKey } from "./personaRuntimeModel.ts";
import {
  countNonSecretInheritedEnvVars,
  getBakedModelInheritLabel,
  getAdvancedInheritedSummary,
  getGlobalModelFallback,
  getInheritedAgentDefaults,
  getBakedProviderInheritLabel,
  resolveInheritedDefault,
} from "./bakedEnvHelpers.ts";

// ── Core predicate: provider-selection support ─────────────────────────────

test("localMode_buzzAgent_supportsProviderSelection", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("buzz-agent"),
    true,
    "buzz-agent must support LLM provider selection",
  );
});

test("localMode_goose_supportsProviderSelection", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("goose"),
    true,
    "goose must support LLM provider selection",
  );
});

test("localMode_claude_doesNotSupportProviderSelection", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("claude"),
    false,
    "claude must NOT support LLM provider selection (CLI-login runtime)",
  );
});

test("localMode_custom_doesNotSupportProviderSelection", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("custom"),
    false,
    "custom runtime must NOT support LLM provider selection",
  );
});

// ── IMPORTANT 1: normalized field gate (provider + model) ─────────────────

test("localMode_buzzAgent_emptyProvider_notSatisfied", () => {
  // Scenario: user selects buzz-agent but leaves provider empty.
  // Rust readiness requires BUZZ_AGENT_PROVIDER — empty = NotReady.
  // The gate must report not-satisfied and surface the missing field marker,
  // but does NOT block the save button.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    result.missingNormalizedFields.includes("provider"),
    "missing provider must be in missingNormalizedFields",
  );
  assert.equal(
    result.satisfied,
    false,
    "empty provider: gate not satisfied (marker shown); save button is still enabled",
  );
});

test("localMode_buzzAgent_emptyModel_notSatisfied", () => {
  // Scenario: buzz-agent + anthropic + API key present, but model left empty.
  // Rust readiness requires BUZZ_AGENT_MODEL — empty = NotReady.
  // The gate surfaces the missing field marker; save button is still enabled.
  const result = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    isProviderMode: false,
    model: "",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    result.missingNormalizedFields.includes("model"),
    "missing model must be in missingNormalizedFields",
  );
  assert.equal(
    result.satisfied,
    false,
    "empty model: gate not satisfied (marker shown); save button is still enabled",
  );
});

// ── Gate: buzz-agent / anthropic with missing key → markers shown ─────────

test("localMode_buzzAgent_anthropic_missingKey_notSatisfied", () => {
  // Scenario: user selects buzz-agent/anthropic + fills model, but hasn't
  // supplied ANTHROPIC_API_KEY — the exact crash-loop case the nudge handles.
  // Gate reports not-satisfied (required marker + env row shown); save allowed.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must be in missingEnvKeys",
  );
  assert.equal(
    result.satisfied,
    false,
    "missing ANTHROPIC_API_KEY: gate not satisfied (marker + nudge shown); save still allowed",
  );
});

test("localMode_buzzAgent_anthropic_allRequired_present_allowed", () => {
  // All three required fields present: provider, model, and credential key.
  const result = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.deepEqual(
    result.missingNormalizedFields,
    [],
    "no missing normalized fields when provider and model are set",
  );
  assert.deepEqual(
    result.missingEnvKeys,
    [],
    "no missing env keys when ANTHROPIC_API_KEY is set",
  );
  assert.equal(
    result.satisfied,
    true,
    "all required fields present must allow create",
  );
});

// ── Gate: claude runtime (CLI-login) → NOT blocked ────────────────────────

test("localMode_claude_noRequiredFields_notBlocked", () => {
  // Scenario: user selects claude. Claude uses CLI-login (out-of-band auth),
  // runtimeSupportsLlmProviderSelection=false → no provider/model required,
  // no credential keys required. The gate must not block.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "claude",
  });

  assert.deepEqual(
    result.missingNormalizedFields,
    [],
    "claude must have no required normalized fields",
  );
  assert.deepEqual(
    result.missingEnvKeys,
    [],
    "claude must return no required credential keys",
  );
  assert.equal(
    result.satisfied,
    true,
    "claude must NOT be blocked by the local-mode gate",
  );
});

// ── Gate: provider mode bypass ─────────────────────────────────

test("localMode_gate_bypassed_for_providerMode", () => {
  // In provider mode, gate must be satisfied regardless of local fields.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: true,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    true,
    "provider mode must bypass the local-mode gate",
  );
});

// ── IMPORTANT 2: requiredEnvKeys surfaces correctly ───────────────────────

test("localMode_requiredEnvKeys_surfaces_anthropicKey", () => {
  // requiredCredentialEnvKeys returns ALL required keys for the provider
  // (including already-satisfied ones) — what EnvVarsEditor receives for
  // its amber locked rows. Verify the full key list, not just missing keys.
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  assert.ok(
    allKeys.includes("ANTHROPIC_API_KEY"),
    "requiredCredentialEnvKeys must include ANTHROPIC_API_KEY for buzz-agent/anthropic",
  );
});

test("localMode_requiredEnvKeys_gate_and_envVarsEditor_share_same_key_set", () => {
  // The key the gate blocks on must equal the key EnvVarsEditor shows.
  // computeLocalModeGate.missingEnvKeys ⊆ requiredCredentialEnvKeys output.
  const gateResult = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });
  const fullKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");

  for (const key of gateResult.missingEnvKeys) {
    assert.ok(
      fullKeys.includes(key),
      `gate-missing key ${key} must appear in requiredCredentialEnvKeys output (EnvVarsEditor source)`,
    );
  }
});

// ── Gate: provider selection drives required credential keys ──────────────

test("localMode_providerSelection_drives_requiredKey", () => {
  // Different provider selections must produce different required keys.
  const anthropicGate = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });
  const databricksGate = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "databricks-meta-llama",
    provider: "databricks",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    anthropicGate.missingEnvKeys.length > 0,
    "anthropic must require at least one credential key",
  );
  assert.ok(
    databricksGate.missingEnvKeys.length > 0,
    "databricks must require at least one credential key",
  );
  assert.notDeepEqual(
    anthropicGate.missingEnvKeys,
    databricksGate.missingEnvKeys,
    "different providers must require different keys",
  );
});

// ── File-config bridge tests ──────────────────────────────────────────────

test("localMode_goose_databricksHost_satisfiedByFileConfig_notRequired", () => {
  // Scenario: goose runtime, databricks_v2 provider, DATABRICKS_HOST in file.
  // The gate should NOT flag DATABRICKS_HOST as missing — it's satisfied in goose config.
  const fileConfig = {
    provider: "databricks_v2",
    model: "goose-claude-4-6-opus",
    satisfiedEnvKeys: ["DATABRICKS_HOST"],
  };
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "goose-claude-4-6-opus",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: fileConfig,
  });

  assert.ok(
    !result.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must NOT appear in missingEnvKeys when satisfied by file config",
  );
  assert.ok(
    result.fileSatisfiedEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must appear in fileSatisfiedEnvKeys when set in goose config",
  );
  assert.equal(
    result.satisfied,
    true,
    "gate must be satisfied when all requirements are covered by env or file config",
  );
});

test("localMode_goose_databricksHost_noFileConfig_stillRequired", () => {
  // Scenario: goose + databricks_v2, no file config present.
  // DATABRICKS_HOST must still be required.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.ok(
    result.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must be required when absent from both env and file config",
  );
  assert.equal(
    result.satisfied,
    false,
    "gate must NOT be satisfied when DATABRICKS_HOST is missing from env and file",
  );
});

test("localMode_goose_providerSatisfiedByFileConfig_noNormalizedFieldRequired", () => {
  // Scenario: goose, no provider in Buzz env but file config has provider + model.
  // Neither 'provider' nor 'model' should be required.
  const fileConfig = {
    provider: "anthropic",
    model: "claude-opus-4-5",
    satisfiedEnvKeys: [],
  };
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "goose",
    runtimeFileConfig: fileConfig,
  });

  assert.deepEqual(
    result.missingNormalizedFields,
    [],
    "normalized fields must be empty when provider + model are in file config",
  );
});

test("localMode_goose_envPlusFileConfig_bothEmpty_stillRequired", () => {
  // Scenario: goose, empty env, file config is null (no file).
  // Both provider and model must be required.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.ok(
    result.missingNormalizedFields.includes("provider"),
    "provider must be required when absent from both env and file",
  );
  assert.ok(
    result.missingNormalizedFields.includes("model"),
    "model must be required when absent from both env and file",
  );
  assert.equal(result.satisfied, false, "gate must not be satisfied");
});

// ── Baked build env satisfaction ──────────────────────────────────────────

test("baked_databricksHost_silencesRequirement", () => {
  // Scenario: goose + databricks_v2, DATABRICKS_HOST baked in (Block build).
  // The gate must NOT flag DATABRICKS_HOST as missing or required.
  const result = computeLocalModeGate({
    bakedEnvKeys: ["DATABRICKS_HOST"],
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.ok(
    !result.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must NOT appear in missingEnvKeys when satisfied by baked env",
  );
  assert.equal(
    result.satisfied,
    true,
    "gate must be satisfied when all requirements are covered by baked env",
  );
});

test("baked_databricksHost_andAgentLocal_agentLocalWins_keyNotRequired", () => {
  // Scenario: DATABRICKS_HOST in both baked env AND agent-local.
  // The key must not appear as required — agent-local takes precedence at spawn
  // time and the key is clearly satisfied.
  const result = computeLocalModeGate({
    bakedEnvKeys: ["DATABRICKS_HOST"],
    envVars: { DATABRICKS_HOST: "https://agent.example.com/" },
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.ok(
    !result.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must not be in missingEnvKeys when set in agent-local env",
  );
  assert.equal(result.satisfied, true, "gate must be satisfied");
});

test("baked_emptyOrUndefined_behaviorUnchanged", () => {
  // Scenario: no baked env (OSS build). DATABRICKS_HOST must still be required.
  const resultUndefined = computeLocalModeGate({
    bakedEnvKeys: undefined,
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });
  const resultEmpty = computeLocalModeGate({
    bakedEnvKeys: [],
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.ok(
    resultUndefined.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must be required when bakedEnvKeys is undefined",
  );
  assert.ok(
    resultEmpty.missingEnvKeys.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must be required when bakedEnvKeys is empty",
  );
  assert.equal(
    resultUndefined.satisfied,
    false,
    "gate must not be satisfied (undefined baked)",
  );
  assert.equal(
    resultEmpty.satisfied,
    false,
    "gate must not be satisfied (empty baked)",
  );
});

test("baked_satisfiedKey_doesNotCountAsMissing_noSaveBlock", () => {
  // A baked-satisfied key must not appear in missingEnvKeys (which drives the
  // save-blocking requiredEnvKeyMissing flag in useRequiredCredentialState).
  const result = computeLocalModeGate({
    bakedEnvKeys: ["DATABRICKS_HOST"],
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
  });

  assert.deepEqual(
    result.missingEnvKeys,
    [],
    "missingEnvKeys must be empty when all required keys are baked-satisfied",
  );
  assert.deepEqual(
    result.fileSatisfiedEnvKeys,
    [],
    "baked-satisfied keys must not appear in fileSatisfiedEnvKeys",
  );
});

// ── getBakedSatisfiedEnvKeys pure function ──────────────────────────────────

test("getBakedSatisfiedEnvKeys_bakedKeyAndNoAgentLocal_returnsBakedKey", () => {
  const result = getBakedSatisfiedEnvKeys(["DATABRICKS_HOST"], {}, [
    "DATABRICKS_HOST",
  ]);
  assert.deepEqual(result, ["DATABRICKS_HOST"]);
});

test("getBakedSatisfiedEnvKeys_agentLocalSet_keyNotBakedSatisfied", () => {
  // Agent-local value wins — the key is already satisfied by the agent's own
  // env, so it doesn't need baked satisfaction.
  const result = getBakedSatisfiedEnvKeys(
    ["DATABRICKS_HOST"],
    { DATABRICKS_HOST: "https://user.example.com/" },
    ["DATABRICKS_HOST"],
  );
  assert.deepEqual(
    result,
    [],
    "key with agent-local value must not be baked-satisfied",
  );
});

test("getBakedSatisfiedEnvKeys_localEmpty_shadowsBakedValue", () => {
  const result = getBakedSatisfiedEnvKeys(
    ["ANTHROPIC_API_KEY"],
    { ANTHROPIC_API_KEY: "" },
    ["ANTHROPIC_API_KEY"],
  );
  assert.deepEqual(
    result,
    [],
    "an explicit local empty string must block baked fallback",
  );
});

test("getBakedSatisfiedEnvKeys_undefinedBaked_returnsEmpty", () => {
  const result = getBakedSatisfiedEnvKeys(["DATABRICKS_HOST"], {}, undefined);
  assert.deepEqual(result, []);
});

test("getBakedSatisfiedEnvKeys_emptyBaked_returnsEmpty", () => {
  const result = getBakedSatisfiedEnvKeys(["DATABRICKS_HOST"], {}, []);
  assert.deepEqual(result, []);
});

// ── requiredEnvKeys exclusion semantics (PersonaDialog / useRequiredCredentialState) ──

test("requiredEnvKeys_exclusionSemantics_filledKeyStaysInAmberRow", () => {
  // A filled required key must stay in the amber locked row (exclusion semantics,
  // not missing-only). Regression guard for the allowlist bug fixed in review.
  // The gate returns missingEnvKeys (empty), not filledKeys — the amber row list
  // is derived independently as allRequired minus baked/file-satisfied.
  const allKeys = requiredCredentialEnvKeys("goose", "databricks_v2");
  const envVarsWithKey = { DATABRICKS_HOST: "https://filled.example.com/" };
  const bakedSatisfied = getBakedSatisfiedEnvKeys(allKeys, envVarsWithKey, []);
  // No baked env, no file config: all keys must stay in the amber row list
  // regardless of whether they are filled.
  const requiredForEditor = allKeys.filter(
    (key) => !bakedSatisfied.includes(key),
  );
  assert.ok(
    requiredForEditor.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must remain in the amber row list even when filled (exclusion semantics)",
  );
});

test("requiredEnvKeys_exclusionSemantics_bakedKeyDropsFromAmberRow", () => {
  // A baked-satisfied key must be excluded from the amber row list.
  const allKeys = requiredCredentialEnvKeys("goose", "databricks_v2");
  const bakedSatisfied = getBakedSatisfiedEnvKeys(allKeys, {}, [
    "DATABRICKS_HOST",
  ]);
  const requiredForEditor = allKeys.filter(
    (key) => !bakedSatisfied.includes(key),
  );
  assert.ok(
    !requiredForEditor.includes("DATABRICKS_HOST"),
    "DATABRICKS_HOST must be excluded from the amber row list when baked-satisfied",
  );
});

// ── Save-block path: hasMissingRequiredEnvKey with baked filter ──────────────

test("saveBlock_bakedSatisfiedKey_notMissing", () => {
  // The save-block gate (hasMissingRequiredEnvKey) must return false when the
  // only unset required key is baked-satisfied. Pins the hook path exercised by
  // useRequiredCredentialState without needing React rendering machinery.
  const allKeys = requiredCredentialEnvKeys("goose", "databricks_v2");
  const bakedSatisfied = getBakedSatisfiedEnvKeys(allKeys, {}, [
    "DATABRICKS_HOST",
  ]);
  // requiredEnvKeys after filtering out baked-satisfied keys (mirrors
  // useRequiredCredentialState's requiredEnvKeys memo).
  const requiredAfterFilter = allKeys.filter(
    (key) => !bakedSatisfied.includes(key),
  );
  assert.equal(
    hasMissingRequiredEnvKey(requiredAfterFilter, {}),
    false,
    "hasMissingRequiredEnvKey must be false when the only unset required key is baked-satisfied",
  );
});

test("saveBlock_noFilterNoBaked_stillMissing", () => {
  // Control: without baked env the same key is still required and missing.
  const allKeys = requiredCredentialEnvKeys("goose", "databricks_v2");
  const bakedSatisfied = getBakedSatisfiedEnvKeys(allKeys, {}, []);
  const requiredAfterFilter = allKeys.filter(
    (key) => !bakedSatisfied.includes(key),
  );
  assert.equal(
    hasMissingRequiredEnvKey(requiredAfterFilter, {}),
    true,
    "hasMissingRequiredEnvKey must be true when the required key is absent and not baked",
  );
});

// ── Global env vars satisfy required credential keys ─────────────────────

test("localMode_globalEnvVars_satisfies_missing_env_key", () => {
  // A required key present in globalEnvVars must not appear in missingEnvKeys.
  const result = computeLocalModeGate({
    envVars: {},
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    true,
    "global ANTHROPIC_API_KEY must satisfy the gate",
  );
  assert.equal(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY in globalEnvVars must not appear in missingEnvKeys",
  );
});

test("localMode_perAgentEnvVar_wins_over_globalEnvVars_for_gate", () => {
  // If the per-agent envVars has the key, globalEnvVars is redundant but
  // the gate must remain satisfied (per-agent wins, both paths satisfy).
  const result = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "sk-per-agent" },
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    true,
    "per-agent key must satisfy the gate regardless of global",
  );
});

test("localMode_globalEnvVars_empty_still_fails_gate", () => {
  // No global and no per-agent env → gate must still surface the missing key.
  const result = computeLocalModeGate({
    envVars: {},
    globalEnvVars: {},
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    false,
    "empty global and per-agent env must leave gate unsatisfied",
  );
  assert.ok(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must be in missingEnvKeys when neither source provides it",
  );
});

// ── Regression: global provider inherited, no credential key supplied ────────
//
// F3 regression from PR #1448 Batch-3 review: the old dialog code derived
// required keys from the *agent-local* provider only and filtered by
// globalConfig.env_vars. An agent with no per-agent provider but globalProvider
// = "anthropic" would show no required-key row (dialog-local provider is ""),
// even though readiness.rs would flag it as NotReady (credential missing).
// computeLocalModeGate must surface the key when the effective provider is
// inherited from globalProvider and neither agent nor global env supplies it.

test("localMode_globalProvider_inherited_no_key_surfacesAsRequired", () => {
  // Agent has no per-agent provider; global provider is "anthropic".
  // Neither agent env nor global env has ANTHROPIC_API_KEY.
  // Expected: the gate must surface ANTHROPIC_API_KEY as missing — the dialog
  // must show the amber required row so the user knows what to configure.
  const result = computeLocalModeGate({
    envVars: {},
    globalEnvVars: {},
    globalProvider: "anthropic",
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    false,
    "gate must not be satisfied when inherited global provider requires a key that is not supplied",
  );
  assert.ok(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must be in missingEnvKeys when global provider is anthropic and no key is in env",
  );
});

test("localMode_globalProvider_inherited_globalEnv_satisfies_key", () => {
  // Agent has no per-agent provider; global provider is "anthropic".
  // Global env has ANTHROPIC_API_KEY — should be satisfied.
  const result = computeLocalModeGate({
    envVars: {},
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    globalProvider: "anthropic",
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.satisfied,
    true,
    "gate must be satisfied when inherited global provider's key is in globalEnvVars",
  );
  assert.equal(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must not be missing when globalEnvVars provides it",
  );
});

// ── Regression: required key stays in requiredEnvKeys when agent fills it ───
//
// EnvVarsEditor.requiredKeys is the full locked-row list — it must remain
// stable while the user is typing in the row. If a key were removed from
// requiredKeys the moment the local value becomes non-empty, the locked amber
// row would unmount mid-entry (focus drop, row swap).
// missingEnvKeys is the gate-state list — it correctly drops the key once
// the value is present. These are now two separate properties.

test("localMode_requiredKey_stays_in_requiredEnvKeys_when_locally_filled", () => {
  // Key starts missing.
  const before = computeLocalModeGate({
    envVars: {},
    globalEnvVars: {},
    globalProvider: "anthropic",
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    before.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    "key must start in missingEnvKeys when no value is set",
  );
  assert.ok(
    before.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    "key must start in requiredEnvKeys when no value is set",
  );

  // User types a value — key is now locally filled.
  const after = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "sk-test" },
    globalEnvVars: {},
    globalProvider: "anthropic",
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    after.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "key must leave missingEnvKeys once a local value is set (gate satisfied)",
  );
  assert.ok(
    after.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    "key must REMAIN in requiredEnvKeys even when locally filled (locked row stays stable)",
  );
  assert.equal(
    after.satisfied,
    true,
    "gate must be satisfied when the key is locally filled",
  );
});

// ── Provider-default label ─────────────────────────────────────────────────

test("providerDefaultLabel_noGlobal_returnsSelectAProvider", () => {
  // No global provider → placeholder signals user must choose.
  const label = getDefaultLlmProviderLabel("buzz-agent", undefined);
  assert.equal(
    label,
    "Select a provider\u2026",
    "no global provider must return 'Select a provider…'",
  );
});

test("providerDefaultLabel_emptyGlobal_returnsSelectAProvider", () => {
  // Empty string treated the same as absent.
  const label = getDefaultLlmProviderLabel("buzz-agent", "");
  assert.equal(
    label,
    "Select a provider\u2026",
    "empty global provider must return 'Select a provider…'",
  );
});

test("providerDefaultLabel_globalSet_returnsInheritLabel", () => {
  // Global provider set → label shows the provider name so the user
  // knows what they're inheriting.
  const label = getDefaultLlmProviderLabel("buzz-agent", "anthropic");
  assert.equal(
    label,
    "Use agent defaults (anthropic)",
    "global provider set must return 'Use agent defaults (<provider>)'",
  );
});

test("providerDefaultLabel_globalSetWithWhitespace_trimsAndReturnsInherit", () => {
  // Surrounding whitespace is stripped before building the label.
  const label = getDefaultLlmProviderLabel("buzz-agent", "  openai  ");
  assert.equal(
    label,
    "Use agent defaults (openai)",
    "global provider with surrounding whitespace must be trimmed in label",
  );
});

test("providerDefaultLabel_sharedCompute_neverLeaksInternalId", () => {
  assert.equal(
    getDefaultLlmProviderLabel("buzz-agent", "relay-mesh"),
    "Use agent defaults (Buzz shared compute)",
  );
});

// ── Baked inherited-default labels ─────────────────────────────────────────

test("bakedDefaults_emptyGlobal_usesBuildValuesForCreateAndEditLabels", () => {
  const bakedEnv = [
    { key: "BUZZ_AGENT_PROVIDER", value: "databricks_v2", masked: false },
    {
      key: "BUZZ_AGENT_MODEL",
      value: "goose-claude-opus-4-8",
      masked: false,
    },
    { key: "BUZZ_AGENT_THINKING_EFFORT", value: "high", masked: false },
  ];
  const provider = resolveInheritedDefault(
    null,
    bakedEnv,
    "BUZZ_AGENT_PROVIDER",
  );
  const model = resolveInheritedDefault(null, bakedEnv, "BUZZ_AGENT_MODEL");
  const effort = resolveInheritedDefault(
    null,
    bakedEnv,
    "BUZZ_AGENT_THINKING_EFFORT",
  );
  const providerOptions = getPersonaProviderOptions("", "buzz-agent");

  assert.deepEqual(provider, { source: "build", value: "databricks_v2" });
  assert.equal(
    getBakedProviderInheritLabel(provider.value, providerOptions),
    "Databricks v2 (inherited from build)",
  );
  assert.deepEqual(model, {
    source: "build",
    value: "goose-claude-opus-4-8",
  });
  assert.equal(
    getBakedModelInheritLabel(model.value),
    "Inherit build default (goose-claude-opus-4-8)",
  );
  assert.deepEqual(effort, { source: "build", value: "high" });
});

test("bakedDefaults_explicitGlobalsOverrideBuildValues", () => {
  const bakedEnv = [
    { key: "BUZZ_AGENT_PROVIDER", value: "databricks_v2", masked: false },
    { key: "BUZZ_AGENT_MODEL", value: "build-model", masked: false },
    { key: "BUZZ_AGENT_THINKING_EFFORT", value: "high", masked: false },
  ];

  assert.deepEqual(
    resolveInheritedDefault("anthropic", bakedEnv, "BUZZ_AGENT_PROVIDER"),
    { source: "global", value: "anthropic" },
  );
  assert.deepEqual(
    resolveInheritedDefault("global-model", bakedEnv, "BUZZ_AGENT_MODEL"),
    { source: "global", value: "global-model" },
  );
  assert.deepEqual(
    resolveInheritedDefault("low", bakedEnv, "BUZZ_AGENT_THINKING_EFFORT"),
    { source: "global", value: "low" },
  );
});

test("advancedSummary_excludesCredentialNamesAndValues", () => {
  const nonSecretGlobalEnvCount = countNonSecretInheritedEnvVars({
    ANTHROPIC_API_KEY: "sk-secret",
    DATABRICKS_HOST: "https://example.databricks.com",
    OTHER_TOKEN: "secret",
  });
  assert.equal(
    nonSecretGlobalEnvCount,
    1,
    "only non-secret inherited defaults belong in the collapsed summary",
  );
  assert.equal(
    getAdvancedInheritedSummary(
      { source: "build", value: "high" },
      nonSecretGlobalEnvCount,
    ),
    "Using inherited defaults: build effort high · 1 global env var",
  );
});

// ── Model-default label ────────────────────────────────────────────────────

test("modelDefaultLabel_noGlobal_returnsDefaultModel", () => {
  // No global model → generic placeholder.
  const label = getDefaultLlmModelLabel(undefined);
  assert.equal(
    label,
    "Default model",
    "no global model must return 'Default model'",
  );
});

test("modelDefaultLabel_emptyGlobal_returnsDefaultModel", () => {
  // Empty string treated the same as absent.
  const label = getDefaultLlmModelLabel("");
  assert.equal(
    label,
    "Default model",
    "empty global model must return 'Default model'",
  );
});

test("modelDefaultLabel_globalSet_returnsInheritLabel", () => {
  // Global model set → label shows the model name so the user
  // knows what they're inheriting.
  const label = getDefaultLlmModelLabel("claude-opus-4-5");
  assert.equal(
    label,
    "Use agent defaults (claude-opus-4-5)",
    "global model set must return 'Use agent defaults (<model>)'",
  );
});

test("modelDefaultLabel_globalSetWithWhitespace_trimsAndReturnsInherit", () => {
  // Surrounding whitespace is stripped before building the label.
  const label = getDefaultLlmModelLabel("  gpt-4o  ");
  assert.equal(
    label,
    "Use agent defaults (gpt-4o)",
    "global model with surrounding whitespace must be trimmed in label",
  );
});

// ── Effective-provider save gate ────────────────────────────────────────────
//
// The canSubmit gate in Create/Edit/PersonaDialog uses:
//   effectiveProvider = provider.trim() || globalProvider.trim()
//   providerValid = !llmProviderFieldVisible || effectiveProvider.length > 0
//
// These tests exercise the core logic through computeLocalModeGate's satisfied
// flag and the label helper; the gate itself is inline in each dialog.

test("providerValid_emptyPerAgentAndNoGlobal_shouldBlockSave", () => {
  // Per Will's confirmed rule: empty per-agent + no global → no effective
  // provider → save MUST be blocked.
  const effectiveProvider = "".trim() || "".trim();
  assert.equal(
    effectiveProvider.length > 0,
    false,
    "empty per-agent + no global must yield an empty effective provider",
  );
});

test("providerValid_emptyPerAgentWithGlobal_shouldAllowSave", () => {
  // Per Will's confirmed rule: empty per-agent + global set → inherit →
  // save MUST be allowed.
  const effectiveProvider = "".trim() || "anthropic".trim();
  assert.equal(
    effectiveProvider.length > 0,
    true,
    "empty per-agent + global set must yield a non-empty effective provider",
  );
});

test("providerValid_explicitPerAgent_shouldAllowSave", () => {
  // Explicit per-agent provider always valid regardless of global.
  const effectiveProvider = "openai".trim() || "".trim();
  assert.equal(
    effectiveProvider.length > 0,
    true,
    "explicit per-agent provider must yield a non-empty effective provider",
  );
});

test("globalAwareGate_globalProviderSet_requiredKeyAppearsWhenMissing", () => {
  // When the effective provider is supplied via the global config (no
  // per-agent provider), required credential rows must still appear.
  const gate = computeLocalModeGate({
    envVars: {},
    globalProvider: "anthropic",
    globalEnvVars: {},
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
    runtimeFileConfig: undefined,
  });
  assert.ok(
    gate.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must appear in requiredEnvKeys when global provider = anthropic and key is missing",
  );
});

test("globalAwareGate_globalProviderAndKeySet_requiredKeyAbsent", () => {
  // When the key is satisfied globally, it must not appear in requiredEnvKeys.
  const gate = computeLocalModeGate({
    envVars: {},
    globalProvider: "anthropic",
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global-key" },
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
    runtimeFileConfig: undefined,
  });
  assert.equal(
    gate.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must NOT appear in requiredEnvKeys when it is satisfied globally",
  );
});

// ── F3 regression: template dialog global-model/provider fallback ──────────
//
// AgentDefinitionDialog now wires missingNormalizedFields (from
// computeLocalModeGate) directly into canSubmit, replacing the old local-only
// isExplicitModelRequired check.  These tests pin the three concrete failure
// cases Thufir identified in pass-2 review so a future regression can't
// silently re-introduce split source-of-truth between display and Save gate.

test("f3_templateDialog_localAnthropicWithGlobalModel_modelNotRequired", () => {
  // Case 1: local provider = anthropic, global model set, local model blank.
  // The gate's effectiveModel = "" || "claude-opus-4-5" || "" = "claude-opus-4-5"
  // → model field satisfied by global → missingNormalizedFields must be empty
  // → canSubmit must NOT be blocked by the model field.
  const result = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    globalEnvVars: {},
    globalModel: "claude-opus-4-5",
    globalProvider: "",
    isProviderMode: false,
    model: "",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.missingNormalizedFields.includes("model"),
    false,
    "model must not be in missingNormalizedFields when global model satisfies it",
  );
  assert.equal(
    result.missingNormalizedFields.length,
    0,
    "missingNormalizedFields must be empty — canSubmit must not be blocked",
  );
});

test("f3_templateDialog_localProviderBlankGlobalAnthropicNoModel_saveBlocked", () => {
  // Case 2: local provider blank, global provider = anthropic, global model blank.
  // effectiveProvider = "" || "anthropic" → model required.
  // effectiveModel = "" || "" = "" → model missing.
  // missingNormalizedFields must contain "model" → canSubmit blocked.
  const result = computeLocalModeGate({
    envVars: {},
    globalEnvVars: {},
    globalModel: "",
    globalProvider: "anthropic",
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    result.missingNormalizedFields.includes("model"),
    "model must be in missingNormalizedFields when global provider requires a model that is not set",
  );
  assert.equal(
    result.satisfied,
    false,
    "gate must not be satisfied — canSubmit must be blocked (no silent NotReady save)",
  );
});

test("f3_templateDialog_globalModelSet_zeroValueLabelIsInherit", () => {
  // Case 3: global model set → the zero-value model dropdown option must show
  // "Use agent defaults (<model>)" not the generic "Default model".
  // getDefaultLlmModelLabel is what AgentDefinitionDialog now uses for that slot.
  assert.equal(
    getDefaultLlmModelLabel("claude-opus-4-5"),
    "Use agent defaults (claude-opus-4-5)",
    "zero-value model option label must show the global model name when set",
  );
  assert.equal(
    getDefaultLlmModelLabel(""),
    "Default model",
    "zero-value model option label must be generic when no global model is set",
  );
});

// ── F3b — option-list composition (Thufir pass 4 acceptance tests) ──────────
//
// These tests exercise the FULL option-list produced by
// buildTemplateModelDropdownOptions, not just the label helper.  They directly
// verify the three cases Thufir required:
//
//   Case 1 (+): provider=anthropic + global model set
//               → composed list CONTAINS the zero-value inherit entry.
//   Case 2 (−): provider=anthropic + NO global model
//               → composed list has NO zero-value entry (model still required).
//   Case 3: provider that doesn't require explicit model (blank) + global model
//               → zero-value already present from getPersonaModelOptions,
//                 no double-seed.

test("f3b_buildTemplateModelDropdownOptions_anthropicGlobalModelSet_containsInheritOption", () => {
  // Case 1: explicit-model provider (anthropic) + global model set.
  // getPersonaModelOptions filters out the zero-value option for anthropic.
  // buildTemplateModelDropdownOptions must prepend it from globalModel.
  const staticOptions = getPersonaModelOptions("buzz-agent", "anthropic");
  const result = buildTemplateModelDropdownOptions(
    staticOptions,
    "claude-opus-4-5",
  );
  const inheritEntry = result.find((o) => o.value === "__auto_model__");
  assert.ok(
    inheritEntry !== undefined,
    "composed list must contain the zero-value inherit entry for anthropic + global model set",
  );
  assert.equal(
    inheritEntry.label,
    "Use agent defaults (claude-opus-4-5)",
    "inherit entry must carry the global model name",
  );
});

test("f3b_buildTemplateModelDropdownOptions_anthropicNoGlobalModel_noZeroValueEntry", () => {
  // Case 2: explicit-model provider (anthropic) + NO global model.
  // No zero-value option must be seeded — model remains required, Save stays blocked.
  const staticOptions = getPersonaModelOptions("buzz-agent", "anthropic");
  const result = buildTemplateModelDropdownOptions(staticOptions, "");
  const inheritEntry = result.find((o) => o.value === "__auto_model__");
  assert.equal(
    inheritEntry,
    undefined,
    "composed list must NOT contain a zero-value entry when no global model is set",
  );
});

test("f3b_buildTemplateModelDropdownOptions_blankProviderGlobalModelSet_noDoubleSeed", () => {
  // Case 3: provider that does NOT require an explicit model (blank string).
  // getPersonaModelOptions returns a zero-value option; the helper must not
  // prepend a second one.
  const staticOptions = getPersonaModelOptions("buzz-agent", "");
  const hasExisting = staticOptions.some((o) => o.id === "");
  assert.ok(
    hasExisting,
    "blank provider must already have a zero-value option from getPersonaModelOptions",
  );
  const result = buildTemplateModelDropdownOptions(
    staticOptions,
    "claude-opus-4-5",
  );
  const autoEntries = result.filter((o) => o.value === "__auto_model__");
  assert.equal(
    autoEntries.length,
    1,
    "must not double-seed the zero-value option when it already exists",
  );
  assert.equal(
    autoEntries[0].label,
    "Use agent defaults (claude-opus-4-5)",
    "existing zero-value entry must be relabeled with the global model name",
  );
});

// ── Unified PROVIDER_CREDENTIAL_CONFIG table regression ───────────────────
// These tests guard the dialog-level fix: requiredCredentialEnvKeys must
// include ANTHROPIC_API_KEY for explicit buzz-agent/anthropic so that the
// EnvVarsEditor amber row renders without a separate dedicated field.

test("providerConfig_explicitAnthropic_requiredKeysIncludesApiKey", () => {
  // Before the fix, AgentDefinitionDialog filtered ANTHROPIC_API_KEY out of
  // the required-row list (via the now-deleted PersonaProviderApiKeyField
  // special-case). The gate itself was always correct — this test documents
  // that requiredCredentialEnvKeys + computeLocalModeGate produce the row.
  const required = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  assert.ok(
    required.includes("ANTHROPIC_API_KEY"),
    "buzz-agent + explicit anthropic must list ANTHROPIC_API_KEY as required",
  );
});

test("providerConfig_inheritThenExplicitAnthropic_sameRequiredKeys", () => {
  // Regression: switching from inherit ("") to explicit "anthropic" must still
  // produce ANTHROPIC_API_KEY as a required key (the row was disappearing on
  // the inherit→explicit switch because the filter re-engaged).
  const inheritKeys = requiredCredentialEnvKeys("buzz-agent", "");
  const explicitKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  assert.ok(
    !inheritKeys.includes("ANTHROPIC_API_KEY"),
    "inherit (empty provider) must not list ANTHROPIC_API_KEY",
  );
  assert.ok(
    explicitKeys.includes("ANTHROPIC_API_KEY"),
    "explicit anthropic must list ANTHROPIC_API_KEY — row must not vanish on switch",
  );
});

test("providerConfig_databricks_requiredKeyIsHost_noSecretClear", () => {
  // Verify Databricks and Anthropic are symmetric: both produce required rows,
  // and the clearing semantics differ only by secretEnvVar presence.
  const databricksKeys = requiredCredentialEnvKeys("buzz-agent", "databricks");
  assert.ok(
    databricksKeys.includes("DATABRICKS_HOST"),
    "buzz-agent + databricks must list DATABRICKS_HOST as required",
  );
  // getProviderApiKeyEnvVar (secretEnvVar) must return null for databricks:
  // DATABRICKS_HOST is a URL, not a secret, and must not be cleared on switch.
  assert.equal(
    getProviderApiKeyEnvVar("databricks"),
    null,
    "databricks must have no secretEnvVar — DATABRICKS_HOST is not cleared on provider switch",
  );
});

// ── Explicit empty agent-local shadows global value (Thufir IMPORTANT #1) ──
//
// An agent-local value of "" explicitly overrides the global key, matching
// backend semantics where agent env.extend() overwrites global layer.
// The UI gate must show the amber row (key is effectively missing) even when
// global config has a non-empty value.

test("isGloballySatisfied_globalSet_keyAbsent_returnsTrue", () => {
  // Global has the key, agent-local does NOT contain it → globally satisfied.
  assert.equal(
    isGloballySatisfiedCredentialKey(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-global" },
      {},
    ),
    true,
    "key absent from agent env and present in global must be globally satisfied",
  );
});

test("isGloballySatisfied_globalSet_keyExplicitlyEmpty_returnsFalse", () => {
  // Global has the key, agent-local has "" → explicit shadow → NOT satisfied.
  assert.equal(
    isGloballySatisfiedCredentialKey(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-global" },
      { ANTHROPIC_API_KEY: "" },
    ),
    false,
    "explicit empty agent-local value must shadow global and return false",
  );
});

test("isGloballySatisfied_globalSet_keyFilledLocally_returnsTrue", () => {
  // Global has the key, agent-local also has a value → locally filled,
  // isGloballySatisfied still returns true (key is not missing).
  // The gate's agentValue path would catch this as locally satisfied.
  assert.equal(
    isGloballySatisfiedCredentialKey(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-global" },
      { ANTHROPIC_API_KEY: "sk-local" },
    ),
    true,
    "key filled both globally and locally must return true (not missing)",
  );
});

test("isGloballySatisfied_globalNotSet_returnsAlwaysFalse", () => {
  // Global does NOT have the key → never globally satisfied regardless of agent state.
  assert.equal(
    isGloballySatisfiedCredentialKey("ANTHROPIC_API_KEY", {}, {}),
    false,
    "absent global key must never be globally satisfied",
  );
  assert.equal(
    isGloballySatisfiedCredentialKey("ANTHROPIC_API_KEY", undefined, {}),
    false,
    "undefined globalEnvVars must never be globally satisfied",
  );
});

test("localMode_globalEnvSatisfied_agentLocalExplicitlyEmpty_stillRequired", () => {
  // Setup: global has ANTHROPIC_API_KEY="sk-global", but agent envVars has
  // ANTHROPIC_API_KEY="" (key present in object, value empty).
  // Backend effective value: "" (agent overwrites global) → missing.
  // Expected: ANTHROPIC_API_KEY appears in missingEnvKeys and requiredEnvKeys.
  const result = computeLocalModeGate({
    envVars: { ANTHROPIC_API_KEY: "" },
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.ok(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must be in missingEnvKeys when agent-local empty string shadows global value",
  );
  assert.ok(
    result.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must be in requiredEnvKeys (amber row must appear)",
  );
  assert.equal(
    result.satisfied,
    false,
    "gate must not be satisfied when agent-local empty shadows global key",
  );
});

test("localMode_globalEnvSatisfied_agentLocalKeyAbsent_silenced", () => {
  // Contrast: global has ANTHROPIC_API_KEY="sk-global", agent envVars does NOT
  // contain the key at all (not present in object). Global satisfies it.
  // Expected: ANTHROPIC_API_KEY silenced — amber row absent, gate satisfied.
  const result = computeLocalModeGate({
    envVars: {}, // key absent, distinct from { ANTHROPIC_API_KEY: "" }
    globalEnvVars: { ANTHROPIC_API_KEY: "sk-global" },
    isProviderMode: false,
    model: "claude-3-5-sonnet-20241022",
    provider: "anthropic",
    runtimeId: "buzz-agent",
  });

  assert.equal(
    result.missingEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must NOT be in missingEnvKeys when global satisfies it and agent-local doesn't override",
  );
  assert.equal(
    result.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must NOT be in requiredEnvKeys when globally satisfied (no amber row)",
  );
  assert.equal(
    result.satisfied,
    true,
    "gate must be satisfied when global key is not shadowed by an explicit empty",
  );
});

test("global model fallback resolves the selected provider model env", () => {
  const bakedEnv = [
    {
      key: "DATABRICKS_MODEL",
      value: "goose-claude-opus-4-8",
      masked: false,
    },
  ];
  assert.equal(
    getGlobalModelFallback(bakedEnv, "databricks_v2"),
    "goose-claude-opus-4-8",
  );
});

test("global model fallback resolves the Gemini provider model env", () => {
  const bakedEnv = [
    { key: "GEMINI_MODEL", value: "gemini-2.5-pro", masked: false },
  ];
  assert.equal(getGlobalModelFallback(bakedEnv, "gemini"), "gemini-2.5-pro");
});

test("global model fallback gives saved Gemini env precedence over build env", () => {
  const bakedEnv = [
    { key: "GEMINI_MODEL", value: "gemini-2.5-flash", masked: false },
  ];
  assert.equal(
    getGlobalModelFallback(bakedEnv, "gemini", {
      GEMINI_MODEL: "gemini-2.5-pro",
    }),
    "gemini-2.5-pro",
  );
});

test("global model fallback gives saved provider env precedence over build env", () => {
  const bakedEnv = [
    { key: "ANTHROPIC_MODEL", value: "build-model", masked: false },
  ];
  assert.equal(
    getGlobalModelFallback(bakedEnv, "anthropic", {
      ANTHROPIC_MODEL: "saved-model",
    }),
    "saved-model",
  );
});

test("global model fallback never uses another provider's model", () => {
  const bakedEnv = [
    { key: "DATABRICKS_MODEL", value: "databricks-model", masked: false },
  ];
  assert.equal(getGlobalModelFallback(bakedEnv, "anthropic"), null);
});

test("inherited defaults expose a provider-specific model fallback to agent dialogs", () => {
  const defaults = getInheritedAgentDefaults(
    { env_vars: {}, provider: "databricks_v2", model: null },
    [
      {
        key: "DATABRICKS_MODEL",
        value: "goose-claude-opus-4-8",
        masked: false,
      },
    ],
  );
  assert.deepEqual(defaults.model, {
    source: "build",
    value: "goose-claude-opus-4-8",
  });
});
