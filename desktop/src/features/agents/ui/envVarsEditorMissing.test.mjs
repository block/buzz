/**
 * Unit tests for isRequiredKeyMissing in EnvVarsEditor.
 *
 * Fix 3 regression: a required env key satisfied by an inherited (global /
 * persona) value must NOT render as missing. Before the fix, isMissing was
 * computed from the agent-local `value[key]` only, so a globally-satisfied key
 * still rendered the amber "Required" badge even though the key was provided.
 *
 * isRequiredKeyMissing is the extracted pure helper that gates the badge.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { isRequiredKeyMissing } from "./EnvVarsEditor.tsx";

// ── Happy paths — key is satisfied ───────────────────────────────────────────

test("envVarsMissing_localValueSet_notMissing", () => {
  // The agent has set the key directly — it is satisfied by the local value.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-local" },
      undefined,
    ),
    false,
    "key present in local value must not be missing",
  );
});

test("envVarsMissing_inheritedValueSet_notMissing", () => {
  // The key is NOT in the agent-local value but IS in inheritedFrom (global).
  // This is the core Fix 3 scenario — the row must NOT show "Required".
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      {},
      { ANTHROPIC_API_KEY: "sk-global" },
    ),
    false,
    "key satisfied by inheritedFrom (global) must not be missing",
  );
});

test("envVarsMissing_bothLocalAndInheritedSet_notMissing", () => {
  // Both sources have the key — local value takes precedence in the UI,
  // but either alone is sufficient for the missing predicate.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-local" },
      { ANTHROPIC_API_KEY: "sk-global" },
    ),
    false,
    "key present in both sources must not be missing",
  );
});

// ── Missing paths — key is genuinely absent ───────────────────────────────────

test("envVarsMissing_neitherLocalNorInherited_isMissing", () => {
  // Neither source provides the key — the badge must show.
  assert.equal(
    isRequiredKeyMissing("ANTHROPIC_API_KEY", {}, undefined),
    true,
    "key absent from both local and inherited must be missing",
  );
});

test("envVarsMissing_inheritedEmpty_isMissing", () => {
  // inheritedFrom exists but the specific key has an empty string value —
  // empty is treated as absent for the missing predicate.
  assert.equal(
    isRequiredKeyMissing("ANTHROPIC_API_KEY", {}, { ANTHROPIC_API_KEY: "" }),
    true,
    "empty inherited value must still be treated as missing",
  );
});

test("envVarsMissing_localEmpty_inheritedUndefined_isMissing", () => {
  // Local value exists but is empty string; no inheritedFrom.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "" },
      undefined,
    ),
    true,
    "empty local value with no inherited must be missing",
  );
});

test("envVarsMissing_differentKey_notInherited_isMissing", () => {
  // inheritedFrom has a different key, not the required one.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      {},
      { OPENAI_API_KEY: "sk-openai" },
    ),
    true,
    "inherited key for a different provider must not satisfy this key",
  );
});
