import assert from "node:assert/strict";
import test from "node:test";

import {
  applyManagedAgentBackendChange,
  backendForChangeIntent,
} from "./changeManagedAgentBackend.ts";

const nodeId = "b".repeat(64);

function localAgent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "Local agent",
    backend: { type: "local" },
    backendAgentId: null,
    status: "stopped",
    ...overrides,
  };
}

test("change intents map onto the ManagedAgentBackend wire shape", () => {
  assert.deepEqual(backendForChangeIntent({ type: "local" }), {
    type: "local",
  });
  assert.deepEqual(backendForChangeIntent({ type: "execution-node", nodeId }), {
    type: "execution_node",
    nodeId,
  });
  assert.deepEqual(
    backendForChangeIntent({ type: "provider", id: "blox", config: { x: 1 } }),
    { type: "provider", id: "blox", config: { x: 1 } },
  );
});

test("swaps first, then deploys, and projects only the confirmed workload", async () => {
  const calls = [];
  const result = await applyManagedAgentBackendChange({
    agent: localAgent(),
    intent: { type: "execution-node", nodeId },
    runtimeId: "goose",
    changeBackend: async (input) => {
      calls.push(["change", input]);
      return {
        ...localAgent(),
        backend: { type: "execution_node", nodeId },
        status: "not_deployed",
      };
    },
    deployExecutionNodeAgent: async (input) => {
      calls.push(["deploy", input]);
      return {
        workloadId: "workload-1",
        receipt: { outcome: { outcome: "succeeded" } },
      };
    },
  });

  assert.equal(calls[0][0], "change");
  assert.deepEqual(calls[0][1], {
    pubkey: "a".repeat(64),
    backend: { type: "execution_node", nodeId },
    runtime: "goose",
    force: false,
  });
  assert.equal(calls[1][0], "deploy");
  assert.deepEqual(calls[1][1], { pubkey: "a".repeat(64), nodeId });
  assert.equal(result.cancelled ?? false, false);
  assert.equal(result.agent.backendAgentId, "workload-1");
  assert.equal(result.agent.status, "deployed");
});

test("a failed or missing receipt rejects instead of projecting a workload", async () => {
  await assert.rejects(
    applyManagedAgentBackendChange({
      agent: localAgent(),
      intent: { type: "execution-node", nodeId },
      runtimeId: "goose",
      changeBackend: async () => ({
        ...localAgent(),
        backend: { type: "execution_node", nodeId },
        status: "not_deployed",
      }),
      deployExecutionNodeAgent: async () => ({ receipt: null }),
    }),
    /execution node rejected the workload command/,
  );
});

test("a local target swaps without deploying", async () => {
  let deployed = false;
  const result = await applyManagedAgentBackendChange({
    agent: localAgent({
      backend: { type: "execution_node", nodeId },
      backendAgentId: "workload-1",
      status: "deployed",
    }),
    intent: { type: "local" },
    changeBackend: async (input) => {
      assert.deepEqual(input.backend, { type: "local" });
      assert.equal(input.runtime, undefined);
      return localAgent();
    },
    deployExecutionNodeAgent: async () => {
      deployed = true;
      return { receipt: null };
    },
  });

  assert.equal(deployed, false);
  assert.equal(result.agent.status, "stopped");
});

test("a deployed provider source requires the orphan confirm and passes force", async () => {
  const prompts = [];
  let changeInput = null;
  await applyManagedAgentBackendChange({
    agent: localAgent({
      backend: { type: "provider", id: "blox", config: {} },
      backendAgentId: "remote-1",
      status: "deployed",
    }),
    intent: { type: "local" },
    changeBackend: async (input) => {
      changeInput = input;
      return localAgent();
    },
    confirmProviderOrphan: (message) => {
      prompts.push(message);
      return true;
    },
  });

  assert.equal(prompts.length, 1);
  assert.match(prompts[0], /abandons it/);
  assert.equal(changeInput.force, true);
});

test("declining the provider orphan confirm cancels without side effects", async () => {
  let changed = false;
  const result = await applyManagedAgentBackendChange({
    agent: localAgent({
      backend: { type: "provider", id: "blox", config: {} },
      backendAgentId: "remote-1",
      status: "deployed",
    }),
    intent: { type: "local" },
    changeBackend: async () => {
      changed = true;
      return localAgent();
    },
    confirmProviderOrphan: () => false,
  });

  assert.deepEqual(result, { cancelled: true });
  assert.equal(changed, false);
});

test("an undeployed provider source never prompts", async () => {
  let prompted = false;
  const result = await applyManagedAgentBackendChange({
    agent: localAgent({
      backend: { type: "provider", id: "blox", config: {} },
      backendAgentId: null,
      status: "not_deployed",
    }),
    intent: { type: "local" },
    changeBackend: async (input) => {
      assert.equal(input.force, false);
      return localAgent();
    },
    confirmProviderOrphan: () => {
      prompted = true;
      return true;
    },
  });

  assert.equal(prompted, false);
  assert.equal(result.agent.backend.type, "local");
});
