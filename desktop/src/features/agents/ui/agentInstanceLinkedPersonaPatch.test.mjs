import assert from "node:assert/strict";
import test from "node:test";

import {
  computeLinkedPersonaProviderModelPatch,
} from "./personaRuntimeModel.ts";

// ── #4157: linked built-in persona provider/model propagation ─────────────
//
// Bug: editing a linked agent's provider or model in the Edit Agent dialog
// silently discards the value for built-in personas because the submit path
// defers provider/model to the persona definition (which ships with both
// unset). The fix propagates the edited values to the definition via
// computeLinkedPersonaProviderModelPatch + useUpdatePersonaMutation.
//
// These tests exercise the pure helper with the same call shape the dialog
// uses, mirroring agentInstanceEditPinning.test.mjs's chain-the-component
// discipline.

test("linked: returns null when local provider and model match persona", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "anthropic",
    model: "claude-sonnet-4-5",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.equal(result, null);
});

test("linked: returns null when local values are empty/whitespace", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "  ",
    model: "   ",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.equal(result, null);
});

test("linked: patches only provider when model is unchanged", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "databricks",
    model: "claude-sonnet-4-5",
    personaProvider: null,
    personaModel: "claude-sonnet-4-5",
  });
  assert.deepEqual(result, { provider: "databricks" });
});

test("linked: patches only model when provider is unchanged", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "anthropic",
    model: "claude-haiku-4-5-20251001",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.deepEqual(result, { model: "claude-haiku-4-5-20251001" });
});

test("linked: patches both when both differ", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "databricks",
    model: "claude-haiku-4-5-20251001",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.deepEqual(result, {
    provider: "databricks",
    model: "claude-haiku-4-5-20251001",
  });
});

test("linked: fills unset persona fields from local edits", () => {
  // Built-in personas ship with provider/model unset — the precise #4157
  // scenario. Any non-empty local edit must propagate.
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: "anthropic",
    model: "claude-sonnet-4-5",
    personaProvider: null,
    personaModel: null,
  });
  assert.deepEqual(result, {
    provider: "anthropic",
    model: "claude-sonnet-4-5",
  });
});

test("linked: trims whitespace before comparing", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: true,
    provider: " anthropic ",
    model: "  claude-sonnet-4-5 ",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.equal(result, null);
});

test("unlinked: always returns null (instance path handles it)", () => {
  const result = computeLinkedPersonaProviderModelPatch({
    hasLinkedPersona: false,
    provider: "databricks",
    model: "claude-sonnet-4-5",
    personaProvider: "anthropic",
    personaModel: "claude-sonnet-4-5",
  });
  assert.equal(result, null);
});
