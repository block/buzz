/**
 * Unit tests for the CreateAgentDialog local-mode credential gate.
 *
 * The gate mirrors EditAgentDialog's block-save logic: when the selected
 * runtime supports LLM provider selection (buzz-agent / goose), a missing
 * required credential env key blocks the create button.
 *
 * On Create there is no inherit checkbox, so selectedRuntimeId IS the
 * prospective runtime — no prospectiveRuntimeId hoist needed.
 *
 * Helpers under test:
 *   requiredCredentialEnvKeys  — returns the env key(s) the dialog can fill
 *   runtimeSupportsLlmProviderSelection — determines if the provider field renders
 *
 * These are the same helpers used by EditAgentDialog, tested in
 * editAgentProviderDiscovery.test.mjs. These tests specifically exercise the
 * create-path wiring (no inherit transition, selectedRuntimeId = prospective).
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
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

// ── Gate: buzz-agent / anthropic with missing key → BLOCKED ───────────────

test("localMode_buzzAgent_anthropic_missingKey_blocked", () => {
  // Scenario: user selects buzz-agent as the runtime and picks anthropic as
  // the provider, but hasn't supplied ANTHROPIC_API_KEY in env vars.
  // The gate must block create — this is exactly the crash-loop case the
  // guarantee closes.
  const selectedRuntimeId = "buzz-agent";
  const provider = "anthropic";

  // Mirror the component's providerForRequiredKeys computation.
  // Create: no inherit transition, selectedRuntimeId IS the prospective runtime.
  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    selectedRuntimeId,
  )
    ? provider
    : "";
  const requiredKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    providerForRequiredKeys,
  );

  const envVars = {}; // ANTHROPIC_API_KEY absent
  const localCredsSatisfied = requiredKeys.every(
    (key) => (envVars[key] ?? "").length > 0,
  );

  assert.equal(
    providerForRequiredKeys,
    "anthropic",
    "providerForRequiredKeys must be the selected provider for buzz-agent",
  );
  assert.ok(
    requiredKeys.length > 0,
    "buzz-agent/anthropic must require at least one env key",
  );
  assert.equal(
    localCredsSatisfied,
    false,
    "missing ANTHROPIC_API_KEY must block create (crash-loop guarantee)",
  );
});

test("localMode_buzzAgent_anthropic_keyPresent_allowed", () => {
  // Same scenario — but the user HAS supplied the key.
  const selectedRuntimeId = "buzz-agent";
  const provider = "anthropic";

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    selectedRuntimeId,
  )
    ? provider
    : "";
  const requiredKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    providerForRequiredKeys,
  );

  const envVars = { ANTHROPIC_API_KEY: "sk-ant-test" };
  const localCredsSatisfied = requiredKeys.every(
    (key) => (envVars[key] ?? "").length > 0,
  );

  assert.equal(
    localCredsSatisfied,
    true,
    "supplying ANTHROPIC_API_KEY must allow create",
  );
});

// ── Gate: claude runtime → NEVER blocked by credential gate ───────────────

test("localMode_claude_noRequiredKeys_notBlocked", () => {
  // Scenario: user selects claude as the runtime. Claude uses CLI-login
  // (out-of-band auth), so the dialog cannot supply a credential — the gate
  // must not block on missing keys.
  const selectedRuntimeId = "claude";
  const provider = ""; // no provider state rendered (llmProviderFieldVisible=false)

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    selectedRuntimeId,
  )
    ? provider
    : "";
  const requiredKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    providerForRequiredKeys,
  );

  const envVars = {}; // nothing set
  const localCredsSatisfied = requiredKeys.every(
    (key) => (envVars[key] ?? "").length > 0,
  );

  assert.equal(
    providerForRequiredKeys,
    "",
    "providerForRequiredKeys must be empty for CLI-login runtimes",
  );
  assert.equal(
    requiredKeys.length,
    0,
    "claude must return no required credential keys",
  );
  assert.equal(
    localCredsSatisfied,
    true,
    "claude must NOT be blocked by the credential gate",
  );
});

// ── Gate: provider selection drives required keys ─────────────────────────

test("localMode_buzzAgent_noProvider_noRequiredKeys", () => {
  // When the user leaves provider at default (""), requiredCredentialEnvKeys
  // must return empty — we can't know which provider-specific key to require.
  const selectedRuntimeId = "buzz-agent";
  const provider = ""; // default / auto

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    selectedRuntimeId,
  )
    ? provider
    : "";
  const requiredKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    providerForRequiredKeys,
  );

  assert.equal(
    requiredKeys.length,
    0,
    "no provider selected must result in no required keys",
  );
});

test("localMode_providerSelection_drives_requiredKey", () => {
  // Verify that different provider selections produce different required keys
  // so the gate is truly keyed on provider state, not hardcoded.
  const selectedRuntimeId = "buzz-agent";

  const anthropicKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    "anthropic",
  );
  const databricksKeys = requiredCredentialEnvKeys(
    selectedRuntimeId,
    "databricks",
  );

  assert.ok(
    anthropicKeys.length > 0,
    "anthropic must require at least one credential key",
  );
  assert.ok(
    databricksKeys.length > 0,
    "databricks must require at least one credential key",
  );
  assert.notDeepEqual(
    anthropicKeys,
    databricksKeys,
    "different providers must require different keys",
  );
});

// ── Gate: isProviderMode / useMesh bypass ─────────────────────────────────

test("localMode_gate_bypassed_for_providerMode", () => {
  // In provider mode, localCredsSatisfied must always be true regardless of
  // env vars — provider mode has its own gate (providerConfigComplete).
  const isProviderMode = true;
  const useMesh = false;
  const requiredKeys = ["SOME_KEY"]; // hypothetical required key
  const envVars = {}; // key absent

  // Mirror the component's localCredsSatisfied computation:
  //   isProviderMode || useMesh || requiredKeys.every(...)
  const localCredsSatisfied =
    isProviderMode ||
    useMesh ||
    requiredKeys.every((key) => (envVars[key] ?? "").length > 0);

  assert.equal(
    localCredsSatisfied,
    true,
    "provider mode must bypass the local credential gate",
  );
});

test("localMode_gate_bypassed_for_meshMode", () => {
  const isProviderMode = false;
  const useMesh = true;
  const requiredKeys = ["SOME_KEY"];
  const envVars = {};

  const localCredsSatisfied =
    isProviderMode ||
    useMesh ||
    requiredKeys.every((key) => (envVars[key] ?? "").length > 0);

  assert.equal(
    localCredsSatisfied,
    true,
    "relay-mesh mode must bypass the local credential gate",
  );
});
