import assert from "node:assert/strict";
import test from "node:test";

import {
  agentAiConfigurationModeSatisfied,
  agentAiConfigurationPairForMode,
  initialAgentAiConfigurationMode,
} from "./agentAiConfigurationPolicy.ts";

test("existing one-sided and complete overrides open in Customize", () => {
  assert.equal(
    initialAgentAiConfigurationMode({ provider: "anthropic" }),
    "custom",
  );
  assert.equal(
    initialAgentAiConfigurationMode({ model: "claude-opus" }),
    "custom",
  );
  assert.equal(
    initialAgentAiConfigurationMode({
      provider: "anthropic",
      model: "claude-opus",
    }),
    "custom",
  );
  assert.equal(initialAgentAiConfigurationMode({}), "defaults");
});

test("Customize requires a complete explicit pair", () => {
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "anthropic",
      model: "",
    }),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "",
      model: "claude-opus",
    }),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "anthropic",
      model: "claude-opus",
    }),
    true,
  );
});

test("Codex/Claude Customize needs only a model, not the hidden provider", () => {
  // needsProviderSelection=false → the intentionally hidden provider must not
  // gate Save (the create/edit "Save stays disabled" regression).
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "gpt-5-codex" },
      false,
    ),
    true,
  );
  // Still needs a model even when the provider is hidden.
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "" },
      false,
    ),
    false,
  );
});

test("Buzz Agent/Goose Customize still requires both provider and model", () => {
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "llama" },
      true,
    ),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "databricks_v2", model: "llama" },
      true,
    ),
    true,
  );
});

test("Defaults clears provider and model together", () => {
  assert.deepEqual(
    agentAiConfigurationPairForMode({
      current: { provider: "anthropic", model: "claude-opus" },
      inherited: { provider: "databricks_v2", model: "llama" },
      mode: "defaults",
    }),
    { provider: "", model: "" },
  );
});

test("entering Customize pins unresolved fields from the inherited pair", () => {
  assert.deepEqual(
    agentAiConfigurationPairForMode({
      current: { provider: "anthropic", model: "" },
      inherited: { provider: "databricks_v2", model: "llama" },
      mode: "custom",
    }),
    { provider: "anthropic", model: "llama" },
  );
});
