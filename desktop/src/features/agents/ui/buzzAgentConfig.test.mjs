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
} from "./buzzAgentConfig.ts";

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

test("openai-compat canonicalizes to openai: empty model returns all-except-max with medium default", () => {
  // Regression: without canonicalization, openai-compat hit the unknown-provider fallback
  // (all 7 values, default medium). After alias, it must resolve identically to openai.
  const compat = getProviderEffortConfig("openai-compat", "");
  const openai = getProviderEffortConfig("openai", "");
  assert.deepEqual([...compat.validValues], [...openai.validValues]);
  assert.equal(compat.defaultValue, openai.defaultValue);
  // Concrete assertion: same as openai unknown model, not the all-7 fallback.
  assert.deepEqual(
    [...compat.validValues],
    ["none", "minimal", "low", "medium", "high", "xhigh"],
  );
  assert.equal(compat.defaultValue, "medium");
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
// getProviderEffortConfig — databricks-v2 hyphen alias (P3-B regression)
// ---------------------------------------------------------------------------

test("databricks-v2 hyphen alias canonicalizes to databricks_v2 underscore records", () => {
  // The persisted alias "databricks-v2" must hit the same canonical records as "databricks_v2".
  const hyphen = getProviderEffortConfig("databricks-v2", "databricks-gpt-5-5");
  const underscore = getProviderEffortConfig(
    "databricks_v2",
    "databricks-gpt-5-5",
  );
  assert.deepEqual([...hyphen.validValues], [...underscore.validValues]);
  assert.equal(hyphen.defaultValue, underscore.defaultValue);
});

test("databricks-v2 hyphen alias with databricks-gpt-5-5 returns [low,medium,high]", () => {
  // Regression: without canonicalization this returned the all-7 unknown-provider fallback.
  const { validValues, defaultValue } = getProviderEffortConfig(
    "databricks-v2",
    "databricks-gpt-5-5",
  );
  assert.deepEqual([...validValues], ["low", "medium", "high"]);
  assert.equal(defaultValue, "medium");
});

// ---------------------------------------------------------------------------
// getProviderEffortConfig — openai-compat alias (Thufir P3 corrective action 1)
// ---------------------------------------------------------------------------
// Rust normalizes "openai-compat" → Provider::OpenAi at config.rs:1218.
// TS PROVIDER_ALIASES must match so UI effort table = Rust request behavior.

test("openai-compat/gpt-5-pro resolves identically to openai/gpt-5-pro", () => {
  const compat = getProviderEffortConfig("openai-compat", "gpt-5-pro");
  const openai = getProviderEffortConfig("openai", "gpt-5-pro");
  assert.deepEqual([...compat.validValues], [...openai.validValues]);
  assert.equal(compat.defaultValue, openai.defaultValue);
  // Concrete: gpt-5-pro is [high] only.
  assert.deepEqual([...compat.validValues], ["high"]);
  assert.equal(compat.defaultValue, "high");
});

test("openai-compat/gpt-5.5 resolves identically to openai/gpt-5.5", () => {
  const compat = getProviderEffortConfig("openai-compat", "gpt-5.5");
  const openai = getProviderEffortConfig("openai", "gpt-5.5");
  assert.deepEqual([...compat.validValues], [...openai.validValues]);
  assert.equal(compat.defaultValue, openai.defaultValue);
});

test("openai-compat/gpt-5 base resolves identically to openai/gpt-5", () => {
  const compat = getProviderEffortConfig("openai-compat", "gpt-5");
  const openai = getProviderEffortConfig("openai", "gpt-5");
  assert.deepEqual([...compat.validValues], [...openai.validValues]);
  assert.equal(compat.defaultValue, openai.defaultValue);
});

test("openai-compat alias is case-insensitive: OpenAI-Compat canonicalizes correctly", () => {
  const mixed = getProviderEffortConfig("OpenAI-Compat", "gpt-5-pro");
  const lower = getProviderEffortConfig("openai-compat", "gpt-5-pro");
  assert.deepEqual([...mixed.validValues], [...lower.validValues]);
  assert.equal(mixed.defaultValue, lower.defaultValue);
});

test("openai-compat alias handles surrounding whitespace: '  openai-compat  ' canonicalizes correctly", () => {
  const padded = getProviderEffortConfig("  openai-compat  ", "gpt-5-pro");
  const clean = getProviderEffortConfig("openai-compat", "gpt-5-pro");
  assert.deepEqual([...padded.validValues], [...clean.validValues]);
  assert.equal(padded.defaultValue, clean.defaultValue);
});

test("openai-compat alias handles mixed case + whitespace: '  OpenAI-Compat  ' canonicalizes correctly", () => {
  const messy = getProviderEffortConfig("  OpenAI-Compat  ", "gpt-5");
  const canonical = getProviderEffortConfig("openai", "gpt-5");
  assert.deepEqual([...messy.validValues], [...canonical.validValues]);
  assert.equal(messy.defaultValue, canonical.defaultValue);
});

// ---------------------------------------------------------------------------
// PROVIDER_FALLBACKS prototype-key safety regression
// ---------------------------------------------------------------------------

test("prototype-key safety: provider='constructor' returns a complete record (Map prevents prototype leak)", () => {
  const result = getProviderEffortConfig("constructor", "some-model");
  // Must not crash and must return a non-empty validValues (complete record)
  assert.ok(
    Array.isArray(result.validValues) && result.validValues.length > 0,
    "constructor provider must return a complete record (not an empty/broken result from prototype chain)",
  );
});

test("prototype-key safety: provider='__proto__' returns a complete record", () => {
  const result = getProviderEffortConfig("__proto__", "some-model");
  assert.ok(
    Array.isArray(result.validValues) && result.validValues.length > 0,
    "__proto__ provider must return a complete record",
  );
});

test("prototype-key safety: provider='hasOwnProperty' returns a complete record", () => {
  const result = getProviderEffortConfig("hasOwnProperty", "some-model");
  assert.ok(
    Array.isArray(result.validValues) && result.validValues.length > 0,
    "hasOwnProperty provider must return a complete record",
  );
});

test("prototype-key safety: provider='toString' returns a complete record", () => {
  const result = getProviderEffortConfig("toString", "some-model");
  assert.ok(
    Array.isArray(result.validValues) && result.validValues.length > 0,
    "toString provider must return a complete record",
  );
});

test("prototype-key safety: validValues.includes() does not throw for prototype-key provider", () => {
  // This exercises the crash path: EffortSelectField calls validValues.includes()
  const result = getProviderEffortConfig("constructor", "some-model");
  assert.doesNotThrow(
    () => result.validValues.includes("low"),
    "validValues.includes() must not throw for prototype-key providers",
  );
});
