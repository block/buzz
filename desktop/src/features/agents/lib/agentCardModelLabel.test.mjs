import assert from "node:assert/strict";
import test from "node:test";

import { resolveAgentCardModelLabel } from "./agentCardModelLabel.ts";

test("resolveAgentCardModelLabel — unspawned definition with explicit model renders the model, not inherited", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: "gpt-5",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "gpt-5");
});

test("resolveAgentCardModelLabel — unspawned definition with no model renders the default", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: null,
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — linked instance inheriting the global default ignores stale persona.model", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "global", model: "stale-model" },
    personaModel: "gpt-5",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — linked instance with no modelSource (legacy/unset) is treated as inherited", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: null, model: "stale-model" },
    personaModel: "gpt-5",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — linked instance with an explicit resolved model renders that model", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "definition", model: "gpt-5" },
    personaModel: "should-not-be-used",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "gpt-5");
});

test("resolveAgentCardModelLabel — non-inherited agent with a blank resolved model falls back to the default", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "instance_legacy", model: "  " },
    personaModel: null,
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

// Databricks registry integration
import { formatAgentModelLabel } from "./formatAgentModelLabel.ts";

test("formatAgentModelLabel — known Databricks managed ID returns curated name", () => {
  assert.equal(formatAgentModelLabel("databricks-gpt-5-5"), "GPT-5.5");
  assert.equal(
    formatAgentModelLabel("databricks-claude-opus-4-7"),
    "Claude Opus 4.7",
  );
});

test("formatAgentModelLabel — unknown custom Databricks ID returns raw ID unchanged", () => {
  assert.equal(
    formatAgentModelLabel("databricks-team-2025-01"),
    "databricks-team-2025-01",
  );
});

test("formatAgentModelLabel — non-Databricks ID returns raw ID unchanged", () => {
  assert.equal(formatAgentModelLabel("claude-sonnet-4-7"), "claude-sonnet-4-7");
  assert.equal(formatAgentModelLabel("gpt-4o"), "gpt-4o");
});

test("formatAgentModelLabel — null or empty returns Auto", () => {
  assert.equal(formatAgentModelLabel(null), "Auto");
  assert.equal(formatAgentModelLabel(""), "Auto");
  assert.equal(formatAgentModelLabel("   "), "Auto");
});

test("resolveAgentCardModelLabel — known Databricks defaultModel renders curated name in default label", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: null,
    defaultModel: "databricks-gpt-5-5",
  });
  assert.equal(label, "Default model (GPT-5.5)");
});

test("resolveAgentCardModelLabel — unknown custom Databricks defaultModel renders raw ID in default label", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: null,
    defaultModel: "databricks-team-2025-01",
  });
  assert.equal(label, "Default model (databricks-team-2025-01)");
});

test("resolveAgentCardModelLabel — known Databricks agent model renders curated name", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "definition", model: "databricks-gpt-oss-120b" },
    personaModel: null,
    defaultModel: "something-else",
  });
  assert.equal(label, "GPT OSS 120B");
});
