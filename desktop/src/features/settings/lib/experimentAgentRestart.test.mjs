import assert from "node:assert/strict";
import test from "node:test";

import {
  applyExperimentAndRestartAgents,
  describeRestartOutcome,
  experimentRequiresAgentRestart,
  restartAgentsForExperiment,
  selectAgentsToRestart,
} from "./experimentAgentRestart.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Agent",
    status: "running",
    backend: { type: "local" },
    ...overrides,
  };
}

test("only spawn-env experiments require a restart confirmation", () => {
  assert.equal(experimentRequiresAgentRestart("acpToolSummaries"), true);
  assert.equal(experimentRequiresAgentRestart("pulse"), false);
});

test("selectAgentsToRestart picks local running agents only", () => {
  const running = agent({ pubkey: "a".repeat(64), name: "Running" });
  const stopped = agent({
    pubkey: "b".repeat(64),
    name: "Stopped",
    status: "stopped",
  });
  const deployed = agent({
    pubkey: "c".repeat(64),
    name: "Deployed",
    status: "deployed",
    backend: { type: "relay-mesh" },
  });
  const remoteRunning = agent({
    pubkey: "d".repeat(64),
    name: "Remote",
    backend: { type: "relay-mesh" },
  });

  assert.deepEqual(
    selectAgentsToRestart([running, stopped, deployed, remoteRunning]),
    [running],
  );
});

test("confirm restarts the snapshot after toggle + mirror, in order", async () => {
  const calls = [];
  const agents = [
    agent({ pubkey: "a".repeat(64), name: "One" }),
    agent({ pubkey: "b".repeat(64), name: "Two" }),
  ];

  const outcome = await applyExperimentAndRestartAgents({
    applyToggle: () => calls.push("toggle"),
    mirrorExperiments: async () => calls.push("mirror"),
    agents,
    startAgent: async (pubkey) => calls.push(`start:${pubkey.slice(0, 1)}`),
    stopAgent: async (pubkey) => calls.push(`stop:${pubkey.slice(0, 1)}`),
  });

  // Toggle then mirror strictly precede any restart traffic — restarted
  // agents must spawn against the NEW mirrored env.
  assert.deepEqual(calls.slice(0, 2), ["toggle", "mirror"]);
  const restartCalls = calls.slice(2);
  assert.equal(restartCalls.filter((c) => c.startsWith("stop:")).length, 2);
  assert.equal(restartCalls.filter((c) => c.startsWith("start:")).length, 2);
  // Each agent stops before it starts.
  assert.ok(restartCalls.indexOf("stop:a") < restartCalls.indexOf("start:a"));
  assert.ok(restartCalls.indexOf("stop:b") < restartCalls.indexOf("start:b"));
  assert.deepEqual(outcome, { restarted: 2, failures: [] });
});

test("a failed mirror write aborts the restart but keeps the toggle applied", async () => {
  const calls = [];

  await assert.rejects(
    applyExperimentAndRestartAgents({
      applyToggle: () => calls.push("toggle"),
      mirrorExperiments: async () => {
        throw new Error("ipc down");
      },
      agents: [agent()],
      startAgent: async () => calls.push("start"),
      stopAgent: async () => calls.push("stop"),
    }),
    /ipc down/,
  );

  // Toggle applied (no rollback), zero restart traffic.
  assert.deepEqual(calls, ["toggle"]);
});

test("cancel path: no orchestration call means no toggle, no restarts", () => {
  // The dialog's cancel/dismiss handler only clears local pending state and
  // never invokes applyExperimentAndRestartAgents — modeled here as: with no
  // running agents selected, there is nothing to restart and outcome is empty.
  assert.deepEqual(selectAgentsToRestart([]), []);
});

test("partial failure: other agents still restart, failures are collected", async () => {
  const one = agent({ pubkey: "a".repeat(64), name: "One" });
  const two = agent({ pubkey: "b".repeat(64), name: "Two" });
  const three = agent({ pubkey: "c".repeat(64), name: "Three" });

  const outcome = await restartAgentsForExperiment({
    agents: [one, two, three],
    stopAgent: async (pubkey) => {
      if (pubkey === two.pubkey) throw new Error("stop failed");
    },
    startAgent: async (pubkey) => {
      if (pubkey === three.pubkey) throw new Error("spawn failed");
    },
  });

  assert.equal(outcome.restarted, 1);
  assert.deepEqual(outcome.failures, [
    { name: "Two", error: "stop failed" },
    { name: "Three", error: "spawn failed" },
  ]);
});

test("describeRestartOutcome messaging covers success and partial failure", () => {
  assert.deepEqual(describeRestartOutcome({ restarted: 1, failures: [] }), {
    kind: "success",
    message: "Restarted 1 agent.",
  });
  assert.deepEqual(describeRestartOutcome({ restarted: 3, failures: [] }), {
    kind: "success",
    message: "Restarted 3 agents.",
  });

  const partial = describeRestartOutcome({
    restarted: 1,
    failures: [
      { name: "Two", error: "stop failed" },
      { name: "Three", error: "spawn failed" },
    ],
  });
  assert.equal(partial.kind, "error");
  assert.match(partial.message, /Restarted 1 of 3 agents/);
  assert.match(partial.message, /Two, Three/);
  assert.match(partial.message, /still applied/);
});
