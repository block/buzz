import assert from "node:assert/strict";
import test from "node:test";

import {
  managedAgentRuntimeLabel,
  runtimeLabel,
} from "./managedAgentRuntimeLabel.ts";

const providerAgent = (id, agentCommand = "codex-acp") => ({
  backend: { type: "provider", id, config: {} },
  agentCommand,
});

test("provider backend wins over a stale Codex harness command", () => {
  assert.equal(managedAgentRuntimeLabel(providerAgent("hermes")), "Hermes");
});

test("unknown provider remains visibly remote", () => {
  assert.equal(
    managedAgentRuntimeLabel(providerAgent("kubernetes")),
    "Remote (kubernetes)",
  );
});

test("local known and custom commands retain their labels", () => {
  assert.equal(runtimeLabel("codex-acp"), "Codex");
  assert.equal(runtimeLabel("hermes"), "Hermes");
  assert.equal(runtimeLabel("/opt/my-agent"), "/opt/my-agent");
});
