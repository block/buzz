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

test("formatAgentModelLabel — Databricks aliases reuse canonical labels", () => {
  assert.equal(
    formatAgentModelLabel("goose-gpt-5-6-sol", "databricks_v2"),
    "GPT-5.6 Sol",
  );
  assert.equal(
    formatAgentModelLabel("goose-claude-fable-5", "databricks_v2"),
    "Claude Fable 5",
  );
  assert.equal(
    formatAgentModelLabel("goose-claude-opus-4-8", "databricks_v2"),
    "Claude Opus 4.8",
  );
  assert.equal(
    formatAgentModelLabel("goose-claude-opus-5", "databricks_v2"),
    "Claude Opus 5",
  );
  assert.equal(
    formatAgentModelLabel("goose-claude-sonnet-5", "databricks_v2"),
    "Claude Sonnet 5",
  );
  assert.equal(
    formatAgentModelLabel("goose-kimi-k3", "databricks_v2"),
    "Kimi K3",
  );
});

test("resolveModelLabel — Databricks alias labels stay provider-scoped", () => {
  assert.equal(
    resolveModelLabel("goose-gpt-5-6-sol", null, "openai"),
    "goose-gpt-5-6-sol",
  );
});

test("formatAgentModelLabel — bare family IDs remain raw", () => {
  assert.equal(formatAgentModelLabel("gpt-5"), "gpt-5");
});

test("formatAgentModelLabel — known Databricks managed ID returns curated name", () => {
  assert.equal(formatAgentModelLabel("databricks-gpt-5-5"), "GPT-5.5");
  assert.equal(
    formatAgentModelLabel("databricks-claude-opus-4-7"),
    "Claude Opus 4.7",
  );
  assert.equal(
    formatAgentModelLabel("databricks-claude-opus-5"),
    "Claude Opus 5",
  );
  assert.equal(
    formatAgentModelLabel("databricks-claude-sonnet-5"),
    "Claude Sonnet 5",
  );
  assert.equal(formatAgentModelLabel("databricks-kimi-k3"), "Kimi K3");
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

import { resolveModelLabel } from "./formatAgentModelLabel.ts";

test("resolveModelLabel — echoed id name falls through to the registry (real discovery shape)", () => {
  // buzz-agent's Databricks discovery emits {id, name: id}; the echoed name
  // carries no display info, so the registry tier must curate the label.
  assert.equal(
    resolveModelLabel(
      "databricks-gpt-5-5",
      "databricks-gpt-5-5",
      "databricks_v2",
    ),
    "GPT-5.5",
  );
});

test("resolveModelLabel — echoed id name for an unknown id stays raw", () => {
  assert.equal(
    resolveModelLabel(
      "databricks-team-2025-01",
      "databricks-team-2025-01",
      "databricks_v2",
    ),
    "databricks-team-2025-01",
  );
});

test("resolveModelLabel — a discovered name distinct from the id still wins tier 1", () => {
  // The "(default catalog)" suffixed name (and any genuinely distinct name) is
  // authoritative and must not be discarded by the echo-equality check.
  assert.equal(
    resolveModelLabel(
      "databricks-gpt-5-5",
      "GPT-5.5 (default catalog)",
      "databricks_v2",
    ),
    "GPT-5.5 (default catalog)",
  );
});

test("resolveAgentCardModelLabel — known Databricks defaultModel with databricks_v2 provider renders curated name in default label", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: null,
    provider: "databricks_v2",
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

// P2 regression: provider-scoped default label — Databricks ID under openai/anthropic must render raw
test("resolveAgentCardModelLabel — openai agent inheriting a Databricks-named default renders raw ID", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "global", model: null, provider: "openai" },
    personaModel: null,
    provider: "openai",
    defaultModel: "databricks-gpt-5-5",
  });
  assert.equal(label, "Default model (databricks-gpt-5-5)");
});

test("resolveAgentCardModelLabel — anthropic agent inheriting a Databricks-named default renders raw ID", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "global", model: null, provider: "anthropic" },
    personaModel: null,
    provider: "anthropic",
    defaultModel: "databricks-gpt-5-5",
  });
  assert.equal(label, "Default model (databricks-gpt-5-5)");
});

test("resolveAgentCardModelLabel — databricks_v2 agent inheriting a Databricks-named default renders curated name", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "global", model: null, provider: "databricks_v2" },
    personaModel: null,
    provider: "databricks_v2",
    defaultModel: "databricks-gpt-5-5",
  });
  assert.equal(label, "Default model (GPT-5.5)");
});

test("resolveAgentCardModelLabel — unspawned openai persona with Databricks-named default renders raw ID", () => {
  const label = resolveAgentCardModelLabel({
    agent: undefined,
    personaModel: null,
    provider: "openai",
    defaultModel: "databricks-gpt-5-5",
  });
  assert.equal(label, "Default model (databricks-gpt-5-5)");
});
