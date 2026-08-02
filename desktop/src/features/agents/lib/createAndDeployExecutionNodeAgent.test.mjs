import assert from "node:assert/strict";
import test from "node:test";

import { createAndDeployExecutionNodeAgent } from "./createAndDeployExecutionNodeAgent.ts";

const baseResponse = {
  agent: {
    pubkey: "a".repeat(64),
    name: "Remote agent",
    backendAgentId: null,
    status: "not_deployed",
  },
  privateKeyNsec: "nsec1private",
  profileSyncError: null,
  spawnError: null,
};

test("creates first and projects only a confirmed execution workload", async () => {
  const calls = [];
  const created = await createAndDeployExecutionNodeAgent({
    input: { name: "Remote agent" },
    createManagedAgent: async (input) => {
      calls.push(["create", input]);
      return baseResponse;
    },
    deployExecutionNodeAgent: async (input) => {
      calls.push(["deploy", input]);
      return {
        workloadId: "workload-1",
        receipt: { outcome: { outcome: "succeeded" } },
      };
    },
    nodeId: "b".repeat(64),
    channelId: "channel-1",
  });

  assert.equal(calls[0][0], "create");
  assert.equal(calls[1][0], "deploy");
  assert.equal(created.agent.backendAgentId, "workload-1");
  assert.equal(created.agent.status, "deployed");
});

test("does not project a failed or missing receipt", async () => {
  await assert.rejects(
    createAndDeployExecutionNodeAgent({
      input: { name: "Remote agent" },
      createManagedAgent: async () => baseResponse,
      deployExecutionNodeAgent: async () => ({ receipt: null }),
      nodeId: "b".repeat(64),
    }),
    /execution node rejected the workload command/,
  );
});

test("does not deploy when managed-agent creation reports a spawn error", async () => {
  let deployed = false;
  await assert.rejects(
    createAndDeployExecutionNodeAgent({
      input: { name: "Remote agent" },
      createManagedAgent: async () => ({
        ...baseResponse,
        spawnError: "creation failed",
      }),
      deployExecutionNodeAgent: async () => {
        deployed = true;
        return { receipt: null };
      },
      nodeId: "b".repeat(64),
    }),
    /creation failed/,
  );
  assert.equal(deployed, false);
});
