/**
 * Unit tests for the CreateAgentDialog local-mode readiness gate.
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
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./personaDialogPickers.tsx";

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
