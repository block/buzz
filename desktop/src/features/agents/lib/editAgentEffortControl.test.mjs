import assert from "node:assert/strict";
import test from "node:test";

import { deriveEditAgentEffortControl } from "./editAgentEffortControl.ts";

function runtime(id, thinkingEnvVar) {
  return {
    id,
    label: id,
    thinkingEnvVar,
  };
}

test("per-agent edit renders Claude effort with its implicit Anthropic provider", () => {
  assert.deepEqual(
    deriveEditAgentEffortControl({
      catalogStatus: "ready",
      envVars: { BUZZ_AGENT_THINKING_EFFORT: "medium" },
      model: "claude-fable-5[1m]",
      runtime: runtime("claude", "BUZZ_ACP_EFFORT"),
      runtimeId: "claude",
    }),
    {
      persistenceKey: "BUZZ_AGENT_THINKING_EFFORT",
      provider: "anthropic",
    },
  );
});

test("per-agent edit keeps Buzz Agent effort descriptor-driven", () => {
  assert.deepEqual(
    deriveEditAgentEffortControl({
      catalogStatus: "ready",
      envVars: {},
      provider: "openai",
      runtime: runtime("buzz-agent", "BUZZ_AGENT_THINKING_EFFORT"),
      runtimeId: "buzz-agent",
    }),
    {
      persistenceKey: "BUZZ_AGENT_THINKING_EFFORT",
      provider: "openai",
    },
  );
});

test("per-agent edit omits effort for Codex and unsupported runtimes", () => {
  for (const selectedRuntime of [
    runtime("codex", null),
    runtime("custom", null),
  ]) {
    assert.equal(
      deriveEditAgentEffortControl({
        catalogStatus: "ready",
        envVars: {},
        runtime: selectedRuntime,
        runtimeId: selectedRuntime.id,
      }),
      undefined,
    );
  }
});

test("per-agent edit does not hide the generic key before catalog readiness", () => {
  assert.equal(
    deriveEditAgentEffortControl({
      catalogStatus: "loading",
      envVars: { BUZZ_AGENT_THINKING_EFFORT: "medium" },
      runtime: runtime("claude", "BUZZ_ACP_EFFORT"),
      runtimeId: "claude",
    }),
    undefined,
  );
});
