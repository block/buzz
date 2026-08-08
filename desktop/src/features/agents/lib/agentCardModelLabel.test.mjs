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
    agent: {
      modelSource: "global",
      model: "stale-model",
      agentCommand: "goose",
    },
    personaModel: "gpt-5",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — linked instance with no modelSource (legacy/unset) is treated as inherited", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: null, model: "stale-model", agentCommand: "goose" },
    personaModel: "gpt-5",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — linked instance with an explicit resolved model renders that model", () => {
  const label = resolveAgentCardModelLabel({
    agent: {
      modelSource: "definition",
      model: "gpt-5",
      agentCommand: "goose",
    },
    personaModel: "should-not-be-used",
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "gpt-5");
});

test("resolveAgentCardModelLabel — non-inherited agent with a blank resolved model falls back to the default", () => {
  const label = resolveAgentCardModelLabel({
    agent: {
      modelSource: "instance_legacy",
      model: "  ",
      agentCommand: "goose",
    },
    personaModel: null,
    defaultModel: "claude-sonnet",
  });
  assert.equal(label, "Default model (claude-sonnet)");
});

test("resolveAgentCardModelLabel — OpenClaw-owned models do not pretend an unset Buzz model is the default", () => {
  const label = resolveAgentCardModelLabel({
    agent: { modelSource: "global", model: null, agentCommand: "openclaw" },
    personaModel: null,
    defaultModel: "",
  });
  assert.equal(label, "Managed by OpenClaw");
});
