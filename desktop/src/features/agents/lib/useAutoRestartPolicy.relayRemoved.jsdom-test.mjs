import assert from "node:assert/strict";
import test from "node:test";

import { restartDriftedAgent } from "./useAutoRestartPolicy.ts";
import { markRelayRemoved } from "../managedAgentRelayCleanup.ts";

const PUBKEY = "cd".repeat(32);

// Auto-restart fires only for a running, drifted, opted-in agent.
async function autoRestartAcrossFetch(relayUrl, duringFetch) {
  const calls = [];
  const ops = {
    listManagedAgents: async () => {
      duringFetch();
      return [
        {
          pubkey: PUBKEY,
          needsRestart: true,
          autoRestartOnConfigChange: true,
          status: "running",
        },
      ];
    },
    stopManagedAgent: async () => calls.push("stop"),
    startManagedAgent: async () => calls.push("start"),
  };
  const result = restartDriftedAgent(PUBKEY, relayUrl, ops);
  return { result, calls };
}

test("an auto-restart whose own relay is removed during its pre-fire fetch starts nothing", async () => {
  const { result, calls } = await autoRestartAcrossFetch(
    "wss://auto-own.example",
    () => markRelayRemoved("wss://auto-own.example"),
  );
  await assert.rejects(result, /relay was removed from this device/);
  assert.deepEqual(calls, ["stop"]);
});

test("removing an unrelated relay during the pre-fire fetch still restarts the agent", async () => {
  const { result, calls } = await autoRestartAcrossFetch(
    "wss://auto-kept.example",
    () => markRelayRemoved("wss://auto-unrelated.example"),
  );
  await result;
  assert.deepEqual(calls, ["stop", "start"]);
});
