import assert from "node:assert/strict";
import test from "node:test";

import { restartManagedAgentPair } from "./managedAgentRuntimeHooks.ts";
import {
  captureActiveTurnsForAgentClear,
  getActiveTurnsForAgent,
  resetActiveAgentTurnsStore,
  syncAgentTurnsFromEvents,
} from "./activeAgentTurnsStore.ts";

const PUBKEY = "deadbeef".repeat(8);
const RELAY = "wss://relay.example";
const status = (lifecycle = "running") => ({
  pubkey: PUBKEY,
  relayUrl: RELAY,
  localSetup: true,
  lifecycle,
});

test("restart preflight refusal leaves old turns alone", async () => {
  const calls = [];
  await assert.rejects(
    restartManagedAgentPair(
      PUBKEY,
      RELAY,
      async (pubkey, relay) => {
        assert.equal(pubkey, PUBKEY);
        assert.equal(relay, RELAY);
        calls.push("native-restart");
        throw new Error("selected provider unavailable");
      },
      () => {
        calls.push("capture-old-turns");
        return () => calls.push("clear");
      },
    ),
    /selected provider unavailable/,
  );
  assert.deepEqual(calls, ["capture-old-turns", "native-restart"]);
});

test("successful Stop with failed replacement retires old turns and reports failure", async () => {
  let cleared = false;
  await assert.rejects(
    restartManagedAgentPair(
      PUBKEY,
      RELAY,
      async () => ({ ...status("failed"), error: "replacement failed" }),
      () => () => {
        cleared = true;
      },
    ),
    /replacement failed/,
  );
  assert.equal(cleared, true);
});

test("native restart clears only captured turns, not replacement turns", async () => {
  resetActiveAgentTurnsStore();
  const event = (turnId, channelId, seq) => ({
    seq,
    timestamp: new Date(Date.now() + seq).toISOString(),
    kind: "turn_started",
    agentIndex: 0,
    channelId,
    sessionId: "session",
    turnId,
    payload: null,
  });
  syncAgentTurnsFromEvents(PUBKEY, [event("old", "old-channel", 1)]);
  const result = await restartManagedAgentPair(
    PUBKEY,
    RELAY,
    async () => {
      syncAgentTurnsFromEvents(PUBKEY, [
        event("replacement", "new-channel", 2),
      ]);
      return status();
    },
    (pubkey) => captureActiveTurnsForAgentClear(pubkey),
  );
  assert.equal(result.lifecycle, "running");
  assert.deepEqual(
    getActiveTurnsForAgent(PUBKEY).map((turn) => turn.channelId),
    ["new-channel"],
  );
  resetActiveAgentTurnsStore();
});
