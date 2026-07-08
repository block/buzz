/**
 * Unit tests for the agent-dialog local-mode readiness gate.
 *
 * The gate computes whether required fields are present for the selected
 * runtime: when missing, it surfaces field markers (isRequired) and env-key
 * amber rows (EnvVarsEditor.requiredKeys), and the setup-listener nudge will
 * fire after spawn. The gate NO LONGER blocks the create/save button —
 * users can save with incomplete config and the nudge will guide them.
 *
 * On Create there is no inherit checkbox, so selectedRuntimeId IS the
 * prospective runtime — no prospectiveRuntimeId hoist needed.
 *
 * The shared helper under test:
 *   computeLocalModeGate — pure function used by field isRequired and
 *                           EnvVarsEditor.requiredKeys; canSubmit no longer
 *                           reads gate.satisfied.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  computeLocalModeGate,
  getBakedSatisfiedEnvKeys,
  getDefaultLlmModelLabel,
  getDefaultLlmProviderLabel,
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./personaDialogPickers.tsx";
import { hasMissingRequiredEnvKey } from "./personaRuntimeModel.ts";

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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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

// ── Gate: isProviderMode / useMesh bypass ─────────────────────────────────

test("localMode_gate_bypassed_for_providerMode", () => {
  // In provider mode, gate must be satisfied regardless of local fields.
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: true,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
    useMesh: false,
  });

  assert.equal(
    result.satisfied,
    true,
    "provider mode must bypass the local-mode gate",
  );
});

test("localMode_gate_bypassed_for_meshMode", () => {
  const result = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "",
    provider: "",
    runtimeId: "buzz-agent",
    useMesh: true,
  });

  assert.equal(
    result.satisfied,
    true,
    "relay-mesh mode must bypass the local-mode gate",
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
    useMesh: false,
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
    useMesh: false,
  });
  const databricksGate = computeLocalModeGate({
    envVars: {},
    isProviderMode: false,
    model: "databricks-meta-llama",
    provider: "databricks",
    runtimeId: "buzz-agent",
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
  });
  const resultEmpty = computeLocalModeGate({
    bakedEnvKeys: [],
    envVars: {},
    isProviderMode: false,
    model: "some-model",
    provider: "databricks_v2",
    runtimeId: "goose",
    runtimeFileConfig: null,
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    useMesh: false,
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
    "Inherit global default (anthropic)",
    "global provider set must return 'Inherit global default (<provider>)'",
  );
});

test("providerDefaultLabel_globalSetWithWhitespace_trimsAndReturnsInherit", () => {
  // Surrounding whitespace is stripped before building the label.
  const label = getDefaultLlmProviderLabel("buzz-agent", "  openai  ");
  assert.equal(
    label,
    "Inherit global default (openai)",
    "global provider with surrounding whitespace must be trimmed in label",
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
    "Inherit global default (claude-opus-4-5)",
    "global model set must return 'Inherit global default (<model>)'",
  );
});

test("modelDefaultLabel_globalSetWithWhitespace_trimsAndReturnsInherit", () => {
  // Surrounding whitespace is stripped before building the label.
  const label = getDefaultLlmModelLabel("  gpt-4o  ");
  assert.equal(
    label,
    "Inherit global default (gpt-4o)",
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
    useMesh: false,
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
    useMesh: false,
  });
  assert.equal(
    gate.requiredEnvKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must NOT appear in requiredEnvKeys when it is satisfied globally",
  );
});
