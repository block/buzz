import assert from "node:assert/strict";
import test from "node:test";

import { requiredCredentialEnvKeys } from "./personaDialogPickers.tsx";
import { hasMissingRequiredEnvKey } from "./personaRuntimeModel.ts";

// These tests cover the Edit Agent required-credential gate behaviour added in
// F1: globally-satisfied credential keys must NOT appear in the required-key
// list and must NOT make `requiredEnvKeyMissing` true.
//
// The logic under test lives in `useRequiredCredentialState` (the filter that
// excludes globally-satisfied keys) and the `hasMissingRequiredEnvKey` check
// that follows. Because the hook uses React + async queries we exercise the
// equivalent pure-function path directly.

// ── helpers ─────────────────────────────────────────────────────────────────

/** Simulate the hook's filtered key list: allRequiredKeys minus globally-satisfied. */
function filterRequiredKeys(allKeys, globalEnvVars) {
  return allKeys.filter((key) => (globalEnvVars[key] ?? "").length === 0);
}

// ── global provider + global API key → not missing, no amber row ─────────

test("editAgent_globalProvider_globalApiKey_noPerAgentEnv_notMissing", () => {
  // Setup: buzz-agent runtime, provider = anthropic (from global), key in globalEnvVars.
  // Expected: requiredEnvKeys is empty, requiredEnvKeyMissing is false.
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  // ANTHROPIC_API_KEY should be in the raw list.
  assert.ok(
    allKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must appear in raw required keys for buzz-agent/anthropic",
  );

  const globalEnvVars = { ANTHROPIC_API_KEY: "sk-global" };
  const perAgentEnvVars = {};

  const filteredKeys = filterRequiredKeys(allKeys, globalEnvVars);
  assert.equal(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must be excluded from filteredKeys when globally satisfied",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    false,
    "requiredEnvKeyMissing must be false when the only required key is globally satisfied",
  );
});

// ── global provider, global API key empty → still missing, amber row shows ──

test("editAgent_globalProvider_globalApiKeyEmpty_stillMissing", () => {
  // Setup: buzz-agent runtime, provider = anthropic (from global), key NOT in globalEnvVars.
  // Expected: ANTHROPIC_API_KEY still required (amber row), requiredEnvKeyMissing true.
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const globalEnvVars = {};
  const perAgentEnvVars = {};

  const filteredKeys = filterRequiredKeys(allKeys, globalEnvVars);
  assert.ok(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must remain in filteredKeys when not globally satisfied",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    true,
    "requiredEnvKeyMissing must be true when required key is absent from both agent and global env",
  );
});

// ── per-agent env satisfies the key (global is bonus) → not missing ───────

test("editAgent_perAgentEnvSatisfiesKey_notMissing", () => {
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const globalEnvVars = {};
  const perAgentEnvVars = { ANTHROPIC_API_KEY: "sk-per-agent" };

  const filteredKeys = filterRequiredKeys(allKeys, globalEnvVars);
  // Key remains in filteredKeys (no global to strip it), but agent env satisfies it.
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    false,
    "requiredEnvKeyMissing must be false when per-agent env satisfies the key",
  );
});

// ── global satisfies one key, other key still missing ────────────────────

test("editAgent_globalSatisfiesOneKey_otherKeyStillMissing", () => {
  // Use openai provider which requires OPENAI_API_KEY only.
  // Globally satisfy it but leave per-agent empty to confirm the logic
  // doesn't accidentally clear unrelated keys.
  const allKeysOpenai = requiredCredentialEnvKeys("buzz-agent", "openai");
  const allKeysAnthropic = requiredCredentialEnvKeys("buzz-agent", "anthropic");

  // Satisfy anthropic globally but test openai path separately.
  const globalEnvVarsWithAnthropic = { ANTHROPIC_API_KEY: "sk-global" };
  const filteredOpenai = filterRequiredKeys(
    allKeysOpenai,
    globalEnvVarsWithAnthropic,
  );

  // OPENAI_API_KEY should still be required (not globally satisfied).
  if (allKeysOpenai.includes("OPENAI_API_KEY")) {
    assert.ok(
      filteredOpenai.includes("OPENAI_API_KEY"),
      "OPENAI_API_KEY must remain required when only ANTHROPIC_API_KEY is globally satisfied",
    );
  }

  // Globally satisfy anthropic key; anthropic path should clear.
  const filteredAnthropic = filterRequiredKeys(
    allKeysAnthropic,
    globalEnvVarsWithAnthropic,
  );
  assert.equal(
    filteredAnthropic.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must be cleared by global env",
  );
});
