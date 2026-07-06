import assert from "node:assert/strict";
import test from "node:test";

import {
  BUZZ_AGENT_MAX_CONTEXT_TOKENS,
  BUZZ_AGENT_MAX_OUTPUT_TOKENS,
  BUZZ_AGENT_MAX_ROUNDS,
  BUZZ_AGENT_THINKING_EFFORT,
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
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
