import assert from "node:assert/strict";
import test from "node:test";

import { dispatchCommandDecision } from "./commandDecisionActions.ts";

const decision = {
  key: "run-1:action-1",
  runId: "run-1",
  actionId: "action-1",
  adviser: "operations",
  coaA: "Complete the readiness review today.",
};

test("queues then sends one stable direction to the Chief of Staff", async () => {
  const updates = [];
  const sent = [];
  const execution = await dispatchCommandDecision(
    {
      decision,
      direction: decision.coaA,
      directionSource: "coa_a",
      now: () => 1000,
    },
    {
      openChief: async () => ({
        pubkey: "a".repeat(64),
        channelId: "00000000-0000-4000-8000-000000000001",
      }),
      send: async (message) => sent.push(message),
      onUpdate: (value) => updates.push(value),
    },
  );

  assert.equal(updates[0].status, "queued");
  assert.equal(execution.status, "queued");
  assert.equal(execution.statusText, "Sent to Chief of Staff.");
  assert.equal(sent.length, 1);
  assert.equal(sent[0].channelId, execution.channelId);
  assert.deepEqual(sent[0].mentionPubkeys, [execution.agentPubkey]);
  assert.match(sent[0].content, /CO DIRECTION run-1:action-1/);
});

test("turns dispatch failure into a visible failed execution", async () => {
  const execution = await dispatchCommandDecision(
    {
      decision,
      direction: "Proceed.",
      directionSource: "user",
      now: () => 1000,
    },
    {
      openChief: async () => {
        throw new Error("secret provider failure");
      },
      send: async () => {},
      onUpdate: () => {},
    },
  );
  assert.equal(execution.status, "failed");
  assert.equal(
    execution.statusText,
    "Could not send the direction to the Chief of Staff.",
  );
});
