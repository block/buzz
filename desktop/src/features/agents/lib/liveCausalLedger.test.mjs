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

test("links a marked terminal replay to its approved proposal", async () => {
  const storage = memoryStorage();
  const persistence = browserLedgerPersistence("owner", storage);
  const ledger = new LiveCausalLedger("owner", persistence);
  const approved = {
    schema: "causal-experiment/v1",
    experimentId: "proposal-1",
    recordedAt: "2026-08-05T00:00:00Z",
    task: { description: "Write the file", sourceMessageId: "message-1" },
    execution: {
      sessionId: "proposal",
      turnId: "pending",
      replayOf: "candidate-1",
    },
    failureFingerprint: "host-write/v1",
    context: {
      codeVersion: "code-1",
      policyVersion: "policy-1",
      modelVersion: "model-1",
      toolVersion: "tool-1",
      environmentVersion: "env-1",
    },
    hypothesis: { cause: "Host bypass", evidenceIds: ["evidence-1"] },
    intervention: {
      remedyId: "remedy-1",
      changedVariable: "tool path",
      successCriteria: "Permission before write",
      approvedAt: "2026-08-05T00:00:00Z",
    },
    result: { outcome: "untested", evidenceIds: [] },
    coverage: { acp_observer: "observed", os_sandbox: "missing" },
    relations: { supports: [], contradicts: [], invalidates: [] },
  };
  const seed = new (await import("./causalLedger.ts")).CausalLedger();
  await persistence.appendEntry(await seed.append(approved));
  await persistence.appendEntry(
    await seed.append({
      ...approved,
      experimentId: "dispatch-1",
      task: { ...approved.task, sourceMessageId: "dispatch-1" },
      execution: {
        sessionId: "replay-dispatch:dispatch-1",
        turnId: "awaiting-agent-session",
        replayOf: approved.experimentId,
      },
      result: { outcome: "untested", evidenceIds: ["message:dispatch-1"] },
    }),
  );
  await ledger.ingest(
    "agent",
    observer(1, "task_captured", {
      description: "[buzz-controlled-replay:proposal-1]\n\nRun it",
      sourceMessageId: "dispatch-1",
    }),
  );
  await ledger.ingest("agent", observer(2, "turn_completed"));
  const replay = (await ledger.entries()).at(-1).experiment;
  assert.equal(replay.execution.replayOf, "proposal-1");
  assert.equal(replay.failureFingerprint, approved.failureFingerprint);
  assert.equal(replay.intervention.changedVariable, "tool path");
  assert.equal(replay.result.outcome, "untested");
});
