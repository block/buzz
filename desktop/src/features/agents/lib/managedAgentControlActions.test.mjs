import assert from "node:assert/strict";
import test from "node:test";

import { startManagedAgentWithRules } from "./managedAgentControlActions.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Mesh Agent",
    personaId: null,
    relayUrl: "ws://localhost:3000",
    acpCommand: "sprout-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    model: "hf://demo/model.gguf",
    mcpToolsets: null,
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: new Date(0).toISOString(),
    updatedAt: new Date(0).toISOString(),
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("relay-mesh agents cannot be manually started without a fresh target", async () => {
  let called = false;
  await assert.rejects(
    startManagedAgentWithRules({
      agent: agent({
        envVars: {
          SPROUT_AGENT_PROVIDER: "openai",
          OPENAI_COMPAT_BASE_URL: "http://127.0.0.1:9337/v1/",
        },
      }),
      startManagedAgent: async () => {
        called = true;
      },
    }),
    /Relay-mesh agents need a fresh serve target/,
  );
  assert.equal(called, false);
});

test("ordinary local agents still start normally", async () => {
  let calledWith = null;
  await startManagedAgentWithRules({
    agent: agent(),
    startManagedAgent: async (pubkey) => {
      calledWith = pubkey;
    },
  });
  assert.equal(calledWith, "deadbeef".repeat(8));
});
