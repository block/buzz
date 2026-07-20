import assert from "node:assert/strict";
import test from "node:test";

import { getDiscoveredPersonaModelOptions } from "./usePersonaModelDiscovery.ts";

function response(overrides = {}) {
  return {
    agentName: "mock",
    agentVersion: "0.0.0",
    models: [],
    agentDefaultModel: null,
    selectedModel: null,
    supportsSwitching: true,
    ...overrides,
  };
}

test("merges the harness's own 'default' catalog entry into the canonical default row", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: "default", name: null, description: null },
        { id: "claude-opus-4-8", name: null, description: null },
        { id: "claude-sonnet-5", name: null, description: null },
      ],
    }),
    "",
  );

  // Exactly one default row (id ""), and no raw "default" entry remains.
  assert.deepEqual(
    options.map((option) => option.id),
    ["", "claude-opus-4-8", "claude-sonnet-5"],
  );
  assert.equal(options[0].label, "Default model");
});

test("default row shows the harness-reported current model when available", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      agentDefaultModel: "gpt-5.5[high]",
      models: [
        { id: "gpt-5.5", name: "GPT-5.5", description: null },
        { id: "gpt-5.4", name: "GPT-5.4", description: null },
      ],
    }),
    "",
  );

  assert.equal(options[0].id, "");
  assert.equal(options[0].label, "Default model (gpt-5.5[high])");
  assert.deepEqual(
    options.slice(1).map((option) => option.id),
    ["gpt-5.5", "gpt-5.4"],
  );
});

test("the 'default' id match is case-insensitive and trimmed", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: " Default ", name: null, description: null },
        { id: "claude-sonnet-5", name: null, description: null },
      ],
    }),
    "",
  );

  assert.deepEqual(
    options.map((option) => option.id),
    ["", "claude-sonnet-5"],
  );
});

test("explicit-model providers get no default row (no harness default entry)", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: "goose-claude-4-6-sonnet", name: null, description: null },
      ],
    }),
    "anthropic",
  );

  assert.deepEqual(
    options.map((option) => option.id),
    ["goose-claude-4-6-sonnet"],
  );
});

test("relay-mesh keeps its automatic routing default row", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [{ id: "llama-3", name: "Llama 3", description: null }],
    }),
    "relay-mesh",
  );

  assert.equal(options[0].id, "");
  assert.equal(options[0].label, "Default (auto)");
});

test("returns null when discovery is unsupported or empty", () => {
  assert.equal(
    getDiscoveredPersonaModelOptions(
      response({ supportsSwitching: false }),
      "",
    ),
    null,
  );
  assert.equal(getDiscoveredPersonaModelOptions(null, ""), null);
});
