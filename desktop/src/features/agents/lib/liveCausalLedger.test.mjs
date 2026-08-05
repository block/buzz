import assert from "node:assert/strict";
import test from "node:test";

import {
  browserLedgerPersistence,
  LiveCausalLedger,
} from "./liveCausalLedger.ts";

function memoryStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
}

function observer(seq, kind, payload = {}) {
  return {
    seq,
    timestamp: `2026-08-05T18:5${seq}:00Z`,
    kind,
    agentIndex: 0,
    channelId: "2db0f46d",
    sessionId: "live-session-1",
    turnId: "turn-1",
    payload,
  };
}

test("automatically closes a live session into an owner-local candidate", async () => {
  const storage = memoryStorage();
  const ledger = new LiveCausalLedger(
    "OWNER",
    browserLedgerPersistence("OWNER", storage),
  );
  await ledger.ingest(
    "agent",
    observer(1, "task_captured", {
      description: "Wire this session into the causal ledger",
      sourceMessageId: "414ccec",
    }),
  );
  await ledger.ingest(
    "agent",
    observer(2, "permission_decision", {
      decision: "allow_once",
    }),
  );
  await ledger.ingest("agent", observer(3, "turn_completed"));

  const [entry] = await ledger.entries();
  assert.equal(entry.experiment.result.outcome, "untested");
  assert.equal(entry.experiment.task.sourceMessageId, "414ccec");
  assert.equal(entry.experiment.coverage.acp_permission_gate, "observed");
  assert.equal(entry.experiment.coverage.host_workspace, "missing");
  assert.equal(entry.experiment.coverage.os_sandbox, "missing");
  assert.ok(storage.getItem(ledger.storageKey));
});

test("restores the candidate after restart and does not duplicate terminal replay", async () => {
  const storage = memoryStorage();
  const first = new LiveCausalLedger(
    "owner",
    browserLedgerPersistence("owner", storage),
  );
  await first.ingest("agent", observer(1, "turn_completed"));

  const restarted = new LiveCausalLedger(
    "owner",
    browserLedgerPersistence("owner", storage),
  );
  await restarted.ingest("agent", observer(1, "turn_completed"));
  assert.equal((await restarted.entries()).length, 1);
});
