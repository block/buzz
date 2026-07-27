import assert from "node:assert/strict";
import test from "node:test";

import {
  isGloballySatisfiedCredentialKey,
  requiredCredentialEnvKeys,
} from "./agentConfigOptions.tsx";
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

/**
 * Simulate the hook's filtered key list: allRequiredKeys minus globally/persona-satisfied.
 * Delegates to `isGloballySatisfiedCredentialKey` — the same helper used by
 * `computeLocalModeGate` and `useRequiredCredentialState` — so this test
 * exercises production semantics rather than a local approximation.
 */
function filterRequiredKeys(
  allKeys,
  globalEnvVars,
  envVars = {},
  personaEnvVars = {},
) {
  return allKeys.filter(
    (key) =>
      !isGloballySatisfiedCredentialKey(key, globalEnvVars, envVars) &&
      !isGloballySatisfiedCredentialKey(key, personaEnvVars, envVars),
  );
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

// ── Explicit empty agent-local shadows global value (Thufir IMPORTANT #1) ──
//
// An agent-local value of "" explicitly overrides the global key.
// Backend semantics: agent env.extend() overwrites global layer; empty = missing.
// The UI gate must match: do NOT silence the amber row when the local value is "".

test("editAgent_globalSatisfied_agentLocalExplicitlyEmpty_stillRequired", () => {
  // Setup: global has ANTHROPIC_API_KEY="sk-global", but agent-local has "".
  // Backend effective value: "" (agent overwrites global) → missing.
  // Expected: ANTHROPIC_API_KEY must remain required (amber row appears).
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const globalEnvVars = { ANTHROPIC_API_KEY: "sk-global" };
  const perAgentEnvVars = { ANTHROPIC_API_KEY: "" };

  const filteredKeys = filterRequiredKeys(
    allKeys,
    globalEnvVars,
    perAgentEnvVars,
  );
  assert.ok(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    "ANTHROPIC_API_KEY must remain required when agent-local explicitly shadows global with empty string",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    true,
    "requiredEnvKeyMissing must be true when agent-local empty shadows global key",
  );
});

test("editAgent_globalSatisfied_agentLocalKeyAbsent_stillSilenced", () => {
  // Contrast: global has ANTHROPIC_API_KEY="sk-global", agent-local does NOT
  // have the key at all (key absent from envVars object).
  // In this case the global satisfies it and the amber row must be absent.
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const globalEnvVars = { ANTHROPIC_API_KEY: "sk-global" };
  const perAgentEnvVars = {}; // key NOT present — differs from explicit ""

  const filteredKeys = filterRequiredKeys(
    allKeys,
    globalEnvVars,
    perAgentEnvVars,
  );
  assert.equal(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must be silenced when global satisfies it and agent-local doesn't override",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    false,
    "requiredEnvKeyMissing must be false when global key is not shadowed",
  );
});

// ── F2: persona-satisfied credential keys ──────────────────────────────────

test("editAgent_personaSatisfied_keyNotRequired", () => {
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const personaEnvVars = { ANTHROPIC_API_KEY: "sk-persona" };

  const filteredKeys = filterRequiredKeys(allKeys, {}, {}, personaEnvVars);
  assert.equal(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    false,
    "persona-satisfied key must be excluded from required keys",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, {}),
    false,
    "requiredEnvKeyMissing must be false when persona satisfies the key",
  );
});

test("editAgent_personaSatisfied_agentLocalEmpty_stillRequired", () => {
  const allKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const personaEnvVars = { ANTHROPIC_API_KEY: "sk-persona" };
  const perAgentEnvVars = { ANTHROPIC_API_KEY: "" };

  const filteredKeys = filterRequiredKeys(
    allKeys,
    {},
    perAgentEnvVars,
    personaEnvVars,
  );
  assert.ok(
    filteredKeys.includes("ANTHROPIC_API_KEY"),
    "explicit local empty must shadow persona value",
  );
  assert.equal(
    hasMissingRequiredEnvKey(filteredKeys, perAgentEnvVars),
    true,
    "requiredEnvKeyMissing must be true when local empty shadows persona",
  );
});
