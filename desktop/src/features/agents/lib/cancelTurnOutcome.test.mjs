import assert from "node:assert/strict";
import test from "node:test";

import { awaitCancelTurnOutcome } from "./cancelTurnOutcome.ts";

function harness(sendCancel = async () => {}) {
  let listener;
  let timeout;
  let unsubscribed = false;
  let timeoutCancelled = false;
  const outcome = awaitCancelTurnOutcome({
    requestId: "request-a",
    channelId: "channel-a",
    subscribe: (fn) => {
      listener = fn;
      return () => {
        unsubscribed = true;
      };
    },
    sendCancel,
    scheduleTimeout: (fn) => {
      timeout = fn;
      return () => {
        timeoutCancelled = true;
      };
    },
  });
  return {
    outcome,
    push: (status, overrides = {}) =>
      listener({
        type: "cancel_turn",
        requestId: "request-a",
        channelId: "channel-a",
        status,
        ...overrides,
      }),
    timeout: () => timeout(),
    assertCleaned: () => {
      assert.equal(unsubscribed, true);
      assert.equal(timeoutCancelled, true);
    },
  };
}

for (const status of ["sent", "no_active_turn", "ambiguous_target"]) {
  test(`stop returns the correlated harness result: ${status}`, async () => {
    const h = harness();
    h.push(status);
    assert.equal(await h.outcome, status);
    h.assertCleaned();
  });
}

test("relay delivery, old harnesses, replay, other channels and model acks cannot confirm a stop", async () => {
  const h = harness();
  h.push("sent", { requestId: undefined });
  h.push("sent", { requestId: "old-request" });
  h.push("sent", { channelId: "channel-b" });
  h.push("sent", { type: "switch_model" });
  h.push("future_status");
  h.timeout();
  assert.equal(await h.outcome, "unconfirmed");
  h.assertCleaned();
});

test("stop unsubscribes and clears timeout after a transport error", async () => {
  const h = harness(async () => {
    throw new Error("transport");
  });
  await assert.rejects(h.outcome, /transport/);
  h.assertCleaned();
});

test("a hung transport cannot block the unconfirmed timeout", async () => {
  const h = harness(() => new Promise(() => {}));
  h.timeout();
  assert.equal(await h.outcome, "unconfirmed");
  h.assertCleaned();
});

test("a harness result can settle before the send promise resolves", async () => {
  const h = harness(() => new Promise(() => {}));
  h.push("sent");
  assert.equal(await h.outcome, "sent");
  h.assertCleaned();
});

test("a late transport rejection does not replace the settled result", async () => {
  let rejectSend;
  const h = harness(
    () =>
      new Promise((_resolve, reject) => {
        rejectSend = reject;
      }),
  );
  h.timeout();
  assert.equal(await h.outcome, "unconfirmed");
  rejectSend(new Error("late transport error"));
  await new Promise((resolve) => setImmediate(resolve));
  h.assertCleaned();
});
