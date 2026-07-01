/**
 * Unit tests for the CreateAgentDialog local-mode readiness gate.
 *
 * The gate mirrors EditAgentDialog's block-save logic: when the selected
 * runtime supports LLM provider selection (buzz-agent / goose), a missing
 * required credential env key OR a missing provider/model normalized field
 * blocks the create button.
 *
 * On Create there is no inherit checkbox, so selectedRuntimeId IS the
 * prospective runtime — no prospectiveRuntimeId hoist needed.
 *
 * The shared helper under test:
 *   computeLocalModeGate — pure function used by canSubmit, field isRequired,
 *                           and EnvVarsEditor.requiredKeys; all three share
 *                           the same predicate.
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

test("localMode_buzzAgent_emptyProvider_blocked", () => {
  // Scenario: user selects buzz-agent but leaves provider empty.
  // Rust readiness requires BUZZ_AGENT_PROVIDER — empty = NotReady.
  // The gate must block create even though no env key is required yet.
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
    "empty provider must block create (Rust readiness requires BUZZ_AGENT_PROVIDER)",
  );
});

test("localMode_buzzAgent_emptyModel_blocked", () => {
  // Scenario: buzz-agent + anthropic + API key present, but model left empty.
  // Rust readiness requires BUZZ_AGENT_MODEL — empty = NotReady.
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
    "empty model must block create (Rust readiness requires BUZZ_AGENT_MODEL)",
  );
});

// ── Gate: buzz-agent / anthropic with missing key → BLOCKED ───────────────

test("localMode_buzzAgent_anthropic_missingKey_blocked", () => {
  // Scenario: user selects buzz-agent/anthropic + fills model, but hasn't
  // supplied ANTHROPIC_API_KEY — the exact crash-loop case the guarantee closes.
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
    "missing ANTHROPIC_API_KEY must block create (crash-loop guarantee)",
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
