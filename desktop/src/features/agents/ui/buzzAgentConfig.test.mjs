import assert from "node:assert/strict";
import test from "node:test";

import {
  BUZZ_AGENT_MAX_CONTEXT_TOKENS,
  BUZZ_AGENT_MAX_OUTPUT_TOKENS,
  BUZZ_AGENT_MAX_ROUNDS,
  BUZZ_AGENT_THINKING_EFFORT,
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
  getProviderEffortConfig,
  isBuzzAgentRuntime,
  normalizeEffortValue,
} from "./buzzAgentConfig.ts";
import { resolveEffortFromEnv } from "../lib/agentConfigCore.ts";
import { applyHarnessNativeEffortChange } from "./buzzAgentModelTuningFields.tsx";

// ---------------------------------------------------------------------------
// Thinking effort values
// ---------------------------------------------------------------------------

test("BUZZ_AGENT_THINKING_EFFORT_VALUES contains exactly the 7 accepted values", () => {
  assert.deepEqual(
    [...BUZZ_AGENT_THINKING_EFFORT_VALUES],
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"],
  );
});

test("BUZZ_AGENT_THINKING_EFFORT_VALUES has no duplicates", () => {
  const set = new Set(BUZZ_AGENT_THINKING_EFFORT_VALUES);
  assert.equal(set.size, BUZZ_AGENT_THINKING_EFFORT_VALUES.length);
});

// ---------------------------------------------------------------------------
// Env var key constants
// ---------------------------------------------------------------------------

test("env var key constants match expected BUZZ_AGENT_* names", () => {
  assert.equal(BUZZ_AGENT_THINKING_EFFORT, "BUZZ_AGENT_THINKING_EFFORT");
  assert.equal(BUZZ_AGENT_MAX_OUTPUT_TOKENS, "BUZZ_AGENT_MAX_OUTPUT_TOKENS");
  assert.equal(BUZZ_AGENT_MAX_CONTEXT_TOKENS, "BUZZ_AGENT_MAX_CONTEXT_TOKENS");
  assert.equal(BUZZ_AGENT_MAX_ROUNDS, "BUZZ_AGENT_MAX_ROUNDS");
});

// ---------------------------------------------------------------------------
// isBuzzAgentRuntime
// ---------------------------------------------------------------------------

test("isBuzzAgentRuntime returns true only for buzz-agent id", () => {
  assert.equal(isBuzzAgentRuntime("buzz-agent"), true);
});

test("isBuzzAgentRuntime returns false for other runtimes", () => {
  assert.equal(isBuzzAgentRuntime("goose"), false);
  assert.equal(isBuzzAgentRuntime("custom"), false);
  assert.equal(isBuzzAgentRuntime(""), false);
  assert.equal(isBuzzAgentRuntime("buzz-agent-v2"), false);
});

// ---------------------------------------------------------------------------
// handleEnvVarChange logic (the field→envVars mapping)
// ---------------------------------------------------------------------------

/**
 * Mirrors the handleEnvVarChange helper in CreateAgentRuntimeFields.
 * Tests this directly without rendering React.
 */
function applyEnvVarChange(envVars, key, value) {
  const next = { ...envVars };
  if (value === "") {
    delete next[key];
  } else {
    next[key] = value;
  }
  return next;
}

test("setting a thinking effort value writes the key into envVars", () => {
  const initial = {};
  const result = applyEnvVarChange(initial, BUZZ_AGENT_THINKING_EFFORT, "high");
  assert.equal(result[BUZZ_AGENT_THINKING_EFFORT], "high");
});

test("clearing thinking effort removes the key so the agent inherits", () => {
  const initial = { [BUZZ_AGENT_THINKING_EFFORT]: "high" };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_THINKING_EFFORT, "");
  assert.equal(Object.hasOwn(result, BUZZ_AGENT_THINKING_EFFORT), false);
});

test("setting max output tokens writes the exact BUZZ_AGENT_MAX_OUTPUT_TOKENS key", () => {
  const initial = {};
  const result = applyEnvVarChange(
    initial,
    BUZZ_AGENT_MAX_OUTPUT_TOKENS,
    "4096",
  );
  assert.equal(result[BUZZ_AGENT_MAX_OUTPUT_TOKENS], "4096");
  // Must not affect other keys
  assert.equal(Object.keys(result).length, 1);
});

test("clearing max output tokens removes the key", () => {
  const initial = { [BUZZ_AGENT_MAX_OUTPUT_TOKENS]: "4096" };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_OUTPUT_TOKENS, "");
  assert.equal(Object.hasOwn(result, BUZZ_AGENT_MAX_OUTPUT_TOKENS), false);
});

test("setting context limit writes the exact BUZZ_AGENT_MAX_CONTEXT_TOKENS key", () => {
  const initial = {};
  const result = applyEnvVarChange(
    initial,
    BUZZ_AGENT_MAX_CONTEXT_TOKENS,
    "100000",
  );
  assert.equal(result[BUZZ_AGENT_MAX_CONTEXT_TOKENS], "100000");
});

test("clearing context limit removes the key", () => {
  const initial = { [BUZZ_AGENT_MAX_CONTEXT_TOKENS]: "100000" };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_CONTEXT_TOKENS, "");
  assert.equal(Object.hasOwn(result, BUZZ_AGENT_MAX_CONTEXT_TOKENS), false);
});

test("setting max rounds writes the exact BUZZ_AGENT_MAX_ROUNDS key", () => {
  const initial = {};
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_ROUNDS, "50");
  assert.equal(result[BUZZ_AGENT_MAX_ROUNDS], "50");
});

test("clearing max rounds removes the key", () => {
  const initial = { [BUZZ_AGENT_MAX_ROUNDS]: "50" };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_ROUNDS, "");
  assert.equal(Object.hasOwn(result, BUZZ_AGENT_MAX_ROUNDS), false);
});

test("changing one field does not disturb other env vars", () => {
  const initial = {
    SOME_OTHER_KEY: "value",
    [BUZZ_AGENT_MAX_OUTPUT_TOKENS]: "2048",
  };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_ROUNDS, "20");
  assert.equal(result.SOME_OTHER_KEY, "value");
  assert.equal(result[BUZZ_AGENT_MAX_OUTPUT_TOKENS], "2048");
  assert.equal(result[BUZZ_AGENT_MAX_ROUNDS], "20");
});

test("clearing one field does not disturb other env vars", () => {
  const initial = {
    SOME_OTHER_KEY: "value",
    [BUZZ_AGENT_MAX_OUTPUT_TOKENS]: "2048",
    [BUZZ_AGENT_MAX_ROUNDS]: "20",
  };
  const result = applyEnvVarChange(initial, BUZZ_AGENT_MAX_ROUNDS, "");
  assert.equal(Object.hasOwn(result, BUZZ_AGENT_MAX_ROUNDS), false);
  assert.equal(result.SOME_OTHER_KEY, "value");
  assert.equal(result[BUZZ_AGENT_MAX_OUTPUT_TOKENS], "2048");
});

test("thinking effort select is bounded: all 7 accepted values are present in the constant", () => {
  const expected = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
  for (const v of expected) {
    assert.ok(
      BUZZ_AGENT_THINKING_EFFORT_VALUES.includes(v),
      `missing value: ${v}`,
    );
  }
  assert.equal(BUZZ_AGENT_THINKING_EFFORT_VALUES.length, expected.length);
});

test("non-numeric string is stored as-is (validation is at the backend)", () => {
  // HTML type=number inputs enforce numeric in the browser; we verify the
  // mapping function itself is not a validator — that's intentional.
  const result = applyEnvVarChange(
    {},
    BUZZ_AGENT_MAX_OUTPUT_TOKENS,
    "not-a-number",
  );
  assert.equal(result[BUZZ_AGENT_MAX_OUTPUT_TOKENS], "not-a-number");
});

// ---------------------------------------------------------------------------
// modelTuningRuntimeId → visibility mapping (regression for Edit dialog path)
// ---------------------------------------------------------------------------

// Mirrors the `isBuzzAgent` derivation in CreateAgentRuntimeFields.
// The point of modelTuningRuntimeId is that the Edit dialog can pass
// prospectiveRuntimeId (the real resolved runtime) while selectedRuntimeId
// carries the "inherit"/"custom" sentinel — the two must not be conflated.

test("isBuzzAgentRuntime(prospectiveRuntimeId) shows fields when Edit resolves buzz-agent even though selectedRuntimeId sentinel is 'inherit'", () => {
  // Simulates Edit dialog state: inheritHarness=true, persona is buzz-agent.
  // selectedRuntimeId would be "inherit" (sentinel for custom-command hiding),
  // but prospectiveRuntimeId correctly resolves to "buzz-agent".
  const selectedRuntimeIdSentinel = "inherit"; // what Edit passes to selectedRuntimeId
  const prospectiveRuntimeId = "buzz-agent"; // what Edit passes to modelTuningRuntimeId

  assert.equal(
    isBuzzAgentRuntime(selectedRuntimeIdSentinel),
    false,
    "sentinel 'inherit' must NOT trigger model-tuning fields",
  );
  assert.equal(
    isBuzzAgentRuntime(prospectiveRuntimeId),
    true,
    "prospectiveRuntimeId 'buzz-agent' MUST trigger model-tuning fields",
  );
});

test("isBuzzAgentRuntime(prospectiveRuntimeId) shows fields when Edit has a pinned buzz-agent (selectedRuntimeId sentinel is also 'inherit')", () => {
  // Simulates Edit dialog with a pinned non-custom runtime:
  // selectedRuntimeId sentinel = "inherit" (non-custom known runtime),
  // prospectiveRuntimeId = "buzz-agent" (selectedRuntime?.id).
  const selectedRuntimeIdSentinel = "inherit";
  const prospectiveRuntimeId = "buzz-agent";

  assert.equal(isBuzzAgentRuntime(prospectiveRuntimeId), true);
  assert.equal(isBuzzAgentRuntime(selectedRuntimeIdSentinel), false);
});

test("isBuzzAgentRuntime(prospectiveRuntimeId) hides fields when Edit resolves to non-buzz-agent", () => {
  // E.g. user switches from buzz-agent to goose in Edit — prospectiveRuntimeId = "goose"
  const prospectiveRuntimeId = "goose";
  assert.equal(isBuzzAgentRuntime(prospectiveRuntimeId), false);
});

test("isBuzzAgentRuntime(prospectiveRuntimeId) hides fields when Edit has no resolved runtime (empty string)", () => {
  // prospectiveRuntimeId falls back to "" when catalog hasn't loaded yet
  assert.equal(isBuzzAgentRuntime(""), false);
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — Anthropic model families
// ---------------------------------------------------------------------------

test("anthropic unknown model returns adaptive full set with high default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-3 model returns manual-budget set with null default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-3-7-sonnet-20250219",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high"]);
  assert.equal(defaultValue, null);
});

test("anthropic claude-opus-4-5 returns manual-budget set with null default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-opus-4-5",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high"]);
  assert.equal(defaultValue, null);
});

test("anthropic claude-opus-4-7 returns xhigh-supporting set with high default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-opus-4-7",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-opus-4-6 returns non-xhigh adaptive set (no xhigh)", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-opus-4-6",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-sonnet-5 returns xhigh-supporting set", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-sonnet-5-20260101",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-sonnet-4-6 returns non-xhigh adaptive set", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-sonnet-4-6",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-mythos-preview returns non-xhigh adaptive set", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-mythos-preview",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "max"]);
  assert.equal(defaultValue, "high");
});

test("anthropic claude-mythos-5 returns xhigh-supporting set", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "anthropic",
    "claude-mythos-5",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — OpenAI model families
// ---------------------------------------------------------------------------

test("openai gpt-5-pro returns only high with high default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-5-pro",
  );
  assert.deepEqual([...validValues], ["high"]);
  assert.equal(defaultValue, "high");
});

test("openai gpt-5.5 returns none/low/medium/high/xhigh with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-5.5",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("openai gpt-5.4 returns same table as gpt-5.5", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-5.4",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("openai gpt-5.1 returns none/low/medium/high with none default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-5.1",
  );
  assert.deepEqual([...validValues], ["none", "low", "medium", "high"]);
  assert.equal(defaultValue, "none");
});

test("openai gpt-5 base returns minimal/low/medium/high with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-5",
  );
  assert.deepEqual([...validValues], ["minimal", "low", "medium", "high"]);
  assert.equal(defaultValue, "medium");
});

test("openai unknown model returns all-except-max with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai",
    "gpt-4o",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("openai empty model returns all-except-max with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig("openai", "");
  assert.deepEqual(
    [...validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

// gpt-5-pro must not match the gpt-5 base bucket
test("openai gpt-5-pro is not matched by gpt-5 base bucket", () => {
  const { validValues } = getProviderEffortConfig("openai", "gpt-5-pro");
  assert.deepEqual(
    [...validValues],
    ["high"],
    "gpt-5-pro must use its own table",
  );
});

// gpt-5.10 must NOT match gpt-5.1 (digit boundary)
test("openai gpt-5.10 is not matched by gpt-5.1 token", () => {
  const { validValues } = getProviderEffortConfig("openai", "gpt-5.10");
  // gpt-5.10 doesn't match any specific family → falls into unknown table
  assert.deepEqual(
    [...validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — DatabricksV2 routing
// ---------------------------------------------------------------------------

test("databricks_v2 with claude-opus-4-7 routes to anthropic xhigh table", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "claude-opus-4-7",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

test("databricks_v2 with databricks-prefixed claude model strips prefix and routes correctly", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "databricks-claude-opus-4-7",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high", "xhigh", "max"]);
  assert.equal(defaultValue, "high");
});

test("databricks_v2 with gpt-5.4 routes to openai gpt-5.5/5.4 table", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "gpt-5.4",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("databricks_v2 with databricks-gpt-5.1 strips prefix and routes to OpenAI gpt-5.1", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "databricks-gpt-5.1",
  );
  assert.deepEqual([...validValues], ["none", "low", "medium", "high"]);
  assert.equal(defaultValue, "none");
});

test("databricks_v2 with concrete non-claude non-gpt model excludes max (MLflow clamps it)", () => {
  // llama-3 routes through MlflowChatCompletions → normalize_effort_for_openai_route
  // → max is clamped to xhigh. Show all-except-max so the UI is honest.
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "llama-3",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("databricks_v2 with databricks-prefixed llama model strips prefix and excludes max", () => {
  const { validValues } = getProviderEffortConfig(
    "databricks_v2",
    "databricks-llama-3",
  );
  assert.ok(!validValues.includes("max"));
});

test("databricks_v2 goose-claude-fable-5 strips goose- prefix and routes to anthropic adaptive+xhigh", () => {
  // "goose-claude-fable-5": first claude- occurrence at index 6 → strips "goose-"
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "goose-claude-fable-5",
  );
  assert.ok(
    validValues.includes("max"),
    "max must be valid for goose-claude-fable-5 (adaptive xhigh model)",
  );
  assert.ok(
    validValues.includes("xhigh"),
    "xhigh must be valid for goose-claude-fable-5",
  );
  assert.equal(defaultValue, "high");
});

test("databricks_v2 goose-gpt-5.5 strips goose- prefix and routes to openai gpt-5.5 table", () => {
  // "goose-gpt-5.5": first gpt- occurrence at index 6 → strips "goose-"
  const { validValues } = getProviderEffortConfig(
    "databricks_v2",
    "goose-gpt-5.5",
  );
  assert.ok(!validValues.includes("max"), "max must be excluded for gpt-5.5");
  assert.ok(validValues.includes("xhigh"), "xhigh must be valid for gpt-5.5");
  assert.ok(
    !validValues.includes("minimal"),
    "minimal is not in gpt-5.5 table",
  );
});

test("databricks_v2 arbitrary prefix team-x-claude-opus-4-7 strips to claude-opus-4-7 and routes to anthropic xhigh", () => {
  // "team-x-claude-opus-4-7": first claude- occurrence at index 7 → strips "team-x-"
  // Same routing result as bare "claude-opus-4-7" — verifies prefix allowlist is not required.
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "team-x-claude-opus-4-7",
  );
  assert.ok(
    validValues.includes("max"),
    "max must be valid: claude-opus-4-7 is adaptive xhigh",
  );
  assert.ok(validValues.includes("xhigh"), "xhigh must be valid");
  assert.equal(defaultValue, "high");
});

test("databricks_v2 with empty model returns all-7 (blank = route unknown)", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks_v2",
    "",
  );
  assert.equal(validValues.length, 7);
  assert.equal(defaultValue, "medium");
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — databricks v1 and other providers
// ---------------------------------------------------------------------------

test("databricks v1 routes like openai unknown (no gpt-5 model)", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks",
    "",
  );
  assert.deepEqual(
    [...validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
  assert.equal(defaultValue, "medium");
});

test("openai-compat returns all-7 with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig(
    "openai-compat",
    "",
  );
  assert.equal(validValues.length, 7);
  assert.equal(defaultValue, "medium");
});

test("empty provider returns all-7 with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig("", "");
  assert.equal(validValues.length, 7);
  assert.equal(defaultValue, "medium");
});

test("unknown provider returns all-7 with medium default", () => {
  const { validValues, defaultValue } = getProviderEffortConfig("goose", "");
  assert.equal(validValues.length, 7);
  assert.equal(defaultValue, "medium");
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — provider case-insensitivity
// ---------------------------------------------------------------------------

test("provider id matching is case-insensitive", () => {
  const lower = getProviderEffortConfig("anthropic", "claude-opus-4-7");
  const upper = getProviderEffortConfig("Anthropic", "claude-opus-4-7");
  assert.deepEqual([...lower.validValues], [...upper.validValues]);
  assert.equal(lower.defaultValue, upper.defaultValue);
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — auto-clear logic (valid-values membership)
// ---------------------------------------------------------------------------

// Tests below verify the validValues membership check that the auto-clear
// useEffect in BuzzAgentModelTuningFields and AgentDefaultsSettingsCard
// use to decide whether to reset the current effort to Inherit.

test("effort max is invalid for OpenAI (should trigger auto-clear)", () => {
  const { validValues } = getProviderEffortConfig("openai", "gpt-5.1");
  assert.ok(!validValues.includes("max"), "max must not be in gpt-5.1 set");
});

test("effort none is invalid for Anthropic adaptive (should trigger auto-clear)", () => {
  const { validValues } = getProviderEffortConfig(
    "anthropic",
    "claude-opus-4-7",
  );
  assert.ok(!validValues.includes("none"), "none must not be in anthropic set");
});

test("effort high is valid for all providers (must not trigger auto-clear)", () => {
  for (const provider of [
    "anthropic",
    "openai",
    "databricks_v2",
    "databricks",
    "openai-compat",
    "",
  ]) {
    const { validValues } = getProviderEffortConfig(provider, "");
    assert.ok(
      validValues.includes("high"),
      `high must be valid for provider "${provider}"`,
    );
  }
});

test("effort none is invalid for anthropic manual-budget (should trigger auto-clear)", () => {
  const { validValues } = getProviderEffortConfig(
    "anthropic",
    "claude-3-7-sonnet-20250219",
  );
  assert.ok(
    !validValues.includes("none"),
    "none must not be in manual-budget set",
  );
});

// ---------------------------------------------------------------------------
// applyHarnessNativeEffortChange — production mutation contract
// ---------------------------------------------------------------------------
//
// These tests exercise the PRODUCTION helper imported from
// buzzAgentModelTuningFields.tsx — the same function called by
// HarnessNativeEffortFields.handleChange and AgentConfigFields' effort onChange.
// Two scope variants:
//   record/persona scope: legacyKey = BUZZ_AGENT_THINKING_EFFORT (atomically deleted)
//   global/onboarding scope: legacyKey = null (Delta 5: never delete foreign native key)
//
// Source: goose `crates/goose-provider-types/src/thinking.rs:277-308`
// Canonical: off|low|medium|high|max (exactly 5 values, no minimal/none/xhigh)

const GOOSE_NATIVE_KEY = "GOOSE_THINKING_EFFORT";
// Forward-declare here so mutation + normalization tests share one definition.
const GOOSE_VALUES = ["off", "low", "medium", "high", "max"];
// Aliases from GOOSE_EFFORT_NORMALIZATION (runtime_metadata.rs). Tests that
// exercise alias resolution must pass this explicitly — absent = no aliases.
const GOOSE_ALIASES = [
  ["none", "off"],
  ["disabled", "off"],
  ["med", "medium"],
  ["xhigh", "max"],
];

test("harness_native_save_writes_native_key_and_deletes_legacy", () => {
  // Legacy-only state (pre-migration Goose record): selecting a value must
  // write the native key and atomically delete the legacy key.
  const initial = { [BUZZ_AGENT_THINKING_EFFORT]: "high" };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    BUZZ_AGENT_THINKING_EFFORT,
    "medium",
  );
  assert.equal(result[GOOSE_NATIVE_KEY], "medium", "native key written");
  assert.equal(
    Object.hasOwn(result, BUZZ_AGENT_THINKING_EFFORT),
    false,
    "legacy key deleted atomically on save",
  );
});

test("harness_native_clear_deletes_both_native_and_legacy_keys", () => {
  // When clear is selected (value=""), both native and legacy keys are removed.
  const initial = {
    [GOOSE_NATIVE_KEY]: "high",
    [BUZZ_AGENT_THINKING_EFFORT]: "xhigh",
  };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    BUZZ_AGENT_THINKING_EFFORT,
    "",
  );
  assert.equal(
    Object.hasOwn(result, GOOSE_NATIVE_KEY),
    false,
    "native key deleted on clear",
  );
  assert.equal(
    Object.hasOwn(result, BUZZ_AGENT_THINKING_EFFORT),
    false,
    "legacy key deleted on clear",
  );
});

test("harness_native_save_does_not_disturb_unrelated_env_vars", () => {
  // Saving effort must not affect unrelated env keys.
  const initial = {
    [BUZZ_AGENT_THINKING_EFFORT]: "medium",
    SOME_OTHER_KEY: "value",
    GOOSE_MAX_TOKENS: "4096",
  };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    BUZZ_AGENT_THINKING_EFFORT,
    "off",
  );
  assert.equal(result.SOME_OTHER_KEY, "value", "unrelated key preserved");
  assert.equal(result.GOOSE_MAX_TOKENS, "4096", "numeric key preserved");
  assert.equal(result[GOOSE_NATIVE_KEY], "off", "native key written");
});

test("harness_native_clear_does_not_disturb_unrelated_env_vars", () => {
  // Clearing effort must not affect unrelated env keys.
  const initial = {
    [GOOSE_NATIVE_KEY]: "high",
    SOME_OTHER_KEY: "value",
    GOOSE_MAX_TOKENS: "4096",
  };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    BUZZ_AGENT_THINKING_EFFORT,
    "",
  );
  assert.equal(
    result.SOME_OTHER_KEY,
    "value",
    "unrelated key preserved after clear",
  );
  assert.equal(
    result.GOOSE_MAX_TOKENS,
    "4096",
    "numeric key preserved after clear",
  );
  assert.equal(
    Object.hasOwn(result, GOOSE_NATIVE_KEY),
    false,
    "native key deleted",
  );
});

test("harness_native_global_scope_save_preserves_buzz_agent_effort", () => {
  // At global/onboarding scope (legacyKey=null), saving Goose effort must NOT
  // delete BUZZ_AGENT_THINKING_EFFORT — it is buzz-agent's own global native key
  // (Delta 5: Goose and buzz-agent global keys coexist independently).
  const initial = {
    [BUZZ_AGENT_THINKING_EFFORT]: "high", // buzz-agent's independent global effort
    SOME_OTHER_KEY: "value",
  };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    null, // global scope: never delete foreign native key
    "medium",
  );
  assert.equal(result[GOOSE_NATIVE_KEY], "medium", "Goose native key written");
  assert.equal(
    result[BUZZ_AGENT_THINKING_EFFORT],
    "high",
    "buzz-agent effort preserved (Delta 5: no foreign-key deletion)",
  );
  assert.equal(result.SOME_OTHER_KEY, "value", "unrelated key preserved");
});

test("harness_native_global_scope_clear_preserves_buzz_agent_effort", () => {
  // At global/onboarding scope, clearing Goose effort (Inherit) must NOT
  // delete BUZZ_AGENT_THINKING_EFFORT (Delta 5).
  const initial = {
    [GOOSE_NATIVE_KEY]: "medium",
    [BUZZ_AGENT_THINKING_EFFORT]: "high",
  };
  const result = applyHarnessNativeEffortChange(
    initial,
    GOOSE_NATIVE_KEY,
    null, // global scope
    "",
  );
  assert.equal(
    Object.hasOwn(result, GOOSE_NATIVE_KEY),
    false,
    "Goose native key cleared",
  );
  assert.equal(
    result[BUZZ_AGENT_THINKING_EFFORT],
    "high",
    "buzz-agent effort preserved on Goose clear",
  );
});

test("harness_native_five_canonical_choices_for_goose", () => {
  // Goose's accepted effort vocabulary is exactly the 5 canonical values.
  // The component offers these as the selectable options (plus "Inherit").
  const GOOSE_ACCEPTED = ["off", "low", "medium", "high", "max"];
  assert.equal(
    GOOSE_ACCEPTED.length,
    5,
    "exactly 5 canonical Goose effort values",
  );
  assert.deepEqual(
    GOOSE_ACCEPTED,
    ["off", "low", "medium", "high", "max"],
    "canonical order: off→low→medium→high→max",
  );
  // Verify minimal is NOT a Goose value (buzz-agent only).
  assert.equal(
    GOOSE_ACCEPTED.includes("minimal"),
    false,
    "minimal is not a Goose value",
  );
  // Verify none is NOT canonical (it is an alias for off).
  assert.equal(
    GOOSE_ACCEPTED.includes("none"),
    false,
    "none is not canonical (alias for off)",
  );
});

test("harness_native_legacy_to_native_migration_at_persona_scope_renders_value", () => {
  // A legacy-only Goose persona (scope="definition") with a valid legacy value
  // must display the normalized canonical value via the legacy fallback path.
  // Exercises resolveEffortFromEnv — the same function HarnessNativeEffortFields
  // calls — so this test pins the actual production read path.

  // Case 1: buzz-agent alias xhigh normalizes to max.
  const result1 = resolveEffortFromEnv(
    { [BUZZ_AGENT_THINKING_EFFORT]: "xhigh" },
    "GOOSE_THINKING_EFFORT",
    BUZZ_AGENT_THINKING_EFFORT,
    GOOSE_VALUES,
    GOOSE_ALIASES,
  );
  assert.equal(
    result1.value,
    "max",
    "xhigh alias normalized to max for display",
  );
  assert.equal(
    result1.legacyConsumed,
    true,
    "legacy key consumed when native absent",
  );

  // Case 2: canonical value passes through unchanged.
  const result2 = resolveEffortFromEnv(
    { [BUZZ_AGENT_THINKING_EFFORT]: "high" },
    "GOOSE_THINKING_EFFORT",
    BUZZ_AGENT_THINKING_EFFORT,
    GOOSE_VALUES,
  );
  assert.equal(result2.value, "high", "canonical legacy value passes through");
  assert.equal(
    result2.legacyConsumed,
    true,
    "legacy key consumed when native absent",
  );
});

test("harness_native_invalid_native_plus_valid_legacy_renders_legacy_value", () => {
  // Invalid native value (e.g. "minimal" — not in Goose vocab) + valid legacy value:
  // resolveEffortFromEnv must skip the invalid native and fall back to the valid legacy.
  // This verifies that the component and deriveAgentConfigFieldModel agree on the read.
  const result = resolveEffortFromEnv(
    {
      GOOSE_THINKING_EFFORT: "minimal", // invalid for Goose
      [BUZZ_AGENT_THINKING_EFFORT]: "high", // valid legacy
    },
    "GOOSE_THINKING_EFFORT",
    BUZZ_AGENT_THINKING_EFFORT,
    GOOSE_VALUES,
  );
  assert.equal(
    result.value,
    "high",
    "invalid native skipped; valid legacy provides the displayed value",
  );
  assert.equal(
    result.legacyConsumed,
    true,
    "legacyConsumed is true when native invalid and legacy valid",
  );
});

test("harness_native_invalid_legacy_renders_empty_and_row_stays_visible", () => {
  // Invalid legacy value only (e.g. "minimal" — not in Goose vocab):
  // resolveEffortFromEnv returns null (control shows Inherit/empty) and
  // legacyConsumed is false (row stays visible as an advanced env row).
  const result = resolveEffortFromEnv(
    { [BUZZ_AGENT_THINKING_EFFORT]: "minimal" },
    "GOOSE_THINKING_EFFORT",
    BUZZ_AGENT_THINKING_EFFORT,
    GOOSE_VALUES,
  );
  assert.equal(
    result.value,
    null,
    "invalid legacy returns null — control shows Inherit",
  );
  assert.equal(
    result.legacyConsumed,
    false,
    "legacyConsumed is false — row stays visible as advanced env row",
  );
});

test("harness_native_global_scope_no_legacy_fallback", () => {
  // At global scope the caller passes null for legacyKey — enforcing the tier boundary.
  // Even if a legacy key exists, resolveEffortFromEnv must not consume it.
  const result = resolveEffortFromEnv(
    {
      GOOSE_THINKING_EFFORT: "",
      [BUZZ_AGENT_THINKING_EFFORT]: "high",
    },
    "GOOSE_THINKING_EFFORT",
    null, // global scope: no legacy fallback
    GOOSE_VALUES,
  );
  assert.equal(
    result.value,
    null,
    "global scope: legacy key ignored when legacyKey=null",
  );
  assert.equal(
    result.legacyConsumed,
    false,
    "legacyConsumed is false at global scope",
  );
});

// ---------------------------------------------------------------------------
// normalizeEffortValue — Goose canonical alias normalization
// ---------------------------------------------------------------------------
//
// Source: goose `crates/goose-provider-types/src/thinking.rs:277-308`
// Canonical: off|low|medium|high|max
// Aliases: none|disabled→off, med→medium, xhigh→max (case-insensitive)
// (GOOSE_VALUES and GOOSE_ALIASES are defined above in the HarnessNativeEffortFields section.)

test("normalizeEffortValue_canonical_values_pass_through", () => {
  for (const v of GOOSE_VALUES) {
    assert.equal(normalizeEffortValue(v, GOOSE_VALUES), v);
  }
});

test("normalizeEffortValue_none_to_off", () => {
  assert.equal(
    normalizeEffortValue("none", GOOSE_VALUES, GOOSE_ALIASES),
    "off",
  );
});

test("normalizeEffortValue_disabled_to_off", () => {
  assert.equal(
    normalizeEffortValue("disabled", GOOSE_VALUES, GOOSE_ALIASES),
    "off",
  );
});

test("normalizeEffortValue_med_to_medium", () => {
  assert.equal(
    normalizeEffortValue("med", GOOSE_VALUES, GOOSE_ALIASES),
    "medium",
  );
});

test("normalizeEffortValue_xhigh_to_max", () => {
  assert.equal(
    normalizeEffortValue("xhigh", GOOSE_VALUES, GOOSE_ALIASES),
    "max",
  );
});

test("normalizeEffortValue_case_insensitive", () => {
  assert.equal(normalizeEffortValue("HIGH", GOOSE_VALUES), "high");
  assert.equal(
    normalizeEffortValue("NONE", GOOSE_VALUES, GOOSE_ALIASES),
    "off",
  );
  assert.equal(
    normalizeEffortValue("XHIGH", GOOSE_VALUES, GOOSE_ALIASES),
    "max",
  );
});

test("normalizeEffortValue_invalid_returns_null", () => {
  assert.equal(normalizeEffortValue("minimal", GOOSE_VALUES), null);
  assert.equal(normalizeEffortValue("unknown", GOOSE_VALUES), null);
});

test("normalizeEffortValue_null_acceptedValues_passthrough", () => {
  // buzz-agent path: no static vocab, pass through raw value unchanged.
  assert.equal(normalizeEffortValue("xhigh", null), "xhigh");
  assert.equal(normalizeEffortValue("minimal", null), "minimal");
});

test("normalizeEffortValue_absent_aliases_means_no_aliases", () => {
  // With no alias table, aliases must NOT resolve — absent means no aliases,
  // not Goose aliases. This pins that the fallback table is truly dead.
  assert.equal(normalizeEffortValue("none", GOOSE_VALUES), null);
  assert.equal(normalizeEffortValue("xhigh", GOOSE_VALUES), null);
  assert.equal(normalizeEffortValue("med", GOOSE_VALUES), null);
});

test("normalizeEffortValue_non_goose_alias_resolves_from_descriptor", () => {
  // A runtime with a non-Goose alias table (ultra→max) must resolve from
  // the supplied descriptor, not from any built-in table. Pins that alias
  // resolution is fully driven by the catalog metadata.
  const customValues = ["low", "medium", "max"];
  const customAliases = [["ultra", "max"]];
  assert.equal(
    normalizeEffortValue("ultra", customValues, customAliases),
    "max",
  );
  // "xhigh" is NOT in this runtime's alias table — must return null.
  assert.equal(
    normalizeEffortValue("xhigh", customValues, customAliases),
    null,
  );
});

// ── Claude Code effort normalization ─────────────────────────────────────────

const CLAUDE_VALUES = ["low", "medium", "high", "xhigh", "max"];
const CLAUDE_ALIASES = []; // Claude Code has no aliases

test("claude_valid_effort_value_passes_through", () => {
  // Claude Code canonical values pass through unchanged.
  for (const v of CLAUDE_VALUES) {
    assert.equal(
      normalizeEffortValue(v, CLAUDE_VALUES, CLAUDE_ALIASES),
      v,
      `canonical value "${v}" must pass through for Claude`,
    );
  }
});

test("claude_no_alias_resolution_for_goose_aliases", () => {
  // Claude has no alias table — Goose aliases (none→off, etc.)
  // must NOT resolve for Claude. But xhigh IS canonical for Claude.
  assert.equal(
    normalizeEffortValue("xhigh", CLAUDE_VALUES, CLAUDE_ALIASES),
    "xhigh",
    "xhigh is canonical for Claude — must pass through unchanged",
  );
  // A Goose-only alias like "none" is not in Claude's vocabulary.
  assert.equal(
    normalizeEffortValue("none", CLAUDE_VALUES, CLAUDE_ALIASES),
    null,
    "Goose alias 'none' must not resolve for Claude (no alias table)",
  );
  assert.equal(
    normalizeEffortValue("off", CLAUDE_VALUES, CLAUDE_ALIASES),
    null,
    "Goose canonical 'off' is not in Claude vocabulary — must return null",
  );
});

test("claude_absent_aliases_means_no_aliases", () => {
  // Passing an empty alias array (not null/undefined) must not fall back
  // to any built-in table — the Goose fallback is dead.
  assert.equal(
    normalizeEffortValue("med", CLAUDE_VALUES, CLAUDE_ALIASES),
    null,
    "Goose alias 'med' must not resolve with empty aliases",
  );
});
