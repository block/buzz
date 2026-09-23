import assert from "node:assert/strict";
import test from "node:test";

import { retransmitPermissionDecision } from "./retransmitPermissionDecision.ts";

const NONCE = "nonce-1";

function frame(overrides = {}) {
  return {
    type: "permission_decision",
    status: "sent",
    requestNonce: NONCE,
    ...overrides,
  };
}

/**
 * Controllable harness mirroring the real wiring: a single-listener pub/sub
 * whose unsubscribe genuinely detaches, a manual retransmit tick, a manual
 * deadline flag, and a send counter. `failSends` rejects the first N send
 * attempts, mirroring a socket that is briefly down.
 */
function harness({ nonce = NONCE, failSends = 0 } = {}) {
  let listener = null;
  let tickCb = null;
  let unsubscribeCalls = 0;
  let cancelRetransmitCalls = 0;
  let sendCalls = 0;
  let expired = false;
  let remainingFailures = failSends;

  const outcome = retransmitPermissionDecision({
    requestNonce: nonce,
    send: () => {
      sendCalls += 1;
      if (remainingFailures > 0) {
        remainingFailures -= 1;
        return Promise.reject(new Error("send failed: socket down"));
      }
      return Promise.resolve();
    },
    subscribe: (fn) => {
      listener = fn;
      return () => {
        unsubscribeCalls += 1;
        listener = null;
      };
    },
    scheduleRetransmit: (cb) => {
      tickCb = cb;
      return () => {
        cancelRetransmitCalls += 1;
        // Mirror clearInterval: a cancelled scheduler fires no more ticks.
        tickCb = null;
      };
    },
    deadlineReached: () => expired,
  });

  return {
    outcome,
    push: (f) => listener?.(f),
    tick: () => tickCb?.(),
    expire: () => {
      expired = true;
    },
    get sendCalls() {
      return sendCalls;
    },
    get unsubscribeCalls() {
      return unsubscribeCalls;
    },
    get cancelRetransmitCalls() {
      return cancelRetransmitCalls;
    },
  };
}

const drainMicrotasks = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

test("retransmitPermissionDecision sends immediately and resolves acked on a matching control_result", async () => {
  const h = harness();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 1, "first send fires immediately");

  h.push(frame());
  assert.equal(await h.outcome, "acked");
  assert.equal(h.unsubscribeCalls, 1, "settles unsubscribe the listener");
  assert.equal(
    h.cancelRetransmitCalls,
    1,
    "settles cancel the retransmit loop",
  );
});

test("retransmitPermissionDecision resends on each tick until acked", async () => {
  const h = harness();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 1);

  h.tick();
  h.tick();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 3, "two ticks resend twice more");

  h.push(frame());
  assert.equal(await h.outcome, "acked");
  // A tick after settle must not resend.
  h.tick();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 3, "no resend after the loop has settled");
});

test("retransmitPermissionDecision stops at the deadline and resolves expired without resending", async () => {
  const h = harness();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 1);

  h.expire();
  h.tick();
  assert.equal(await h.outcome, "expired");
  assert.equal(h.sendCalls, 1, "a tick past the deadline must not resend");
  assert.equal(h.unsubscribeCalls, 1);
  assert.equal(h.cancelRetransmitCalls, 1);
});

test("retransmitPermissionDecision resolves acked on an already_decided status", async () => {
  // A late retransmit the harness recognizes as an already-applied duplicate
  // acks `already_decided`; it settles the loop exactly like `sent`.
  const h = harness();
  h.push(frame({ status: "already_decided" }));
  assert.equal(await h.outcome, "acked");
});

test("retransmitPermissionDecision ignores a control_result for a different nonce", async () => {
  const h = harness();
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  // Foreign nonce and a non-permission frame must both be inert.
  h.push(frame({ requestNonce: "other-nonce" }));
  h.push({ type: "switch_model", status: "switched", requestNonce: NONCE });
  await drainMicrotasks();
  assert.equal(settled, false, "no foreign or off-type frame settles the loop");

  h.push(frame());
  assert.equal(await h.outcome, "acked");
});

test("retransmitPermissionDecision survives a rejected first send and acks when a later tick's send resolves", async () => {
  // The causal case: the owner clicks while the relay socket is down. The first
  // send rejects, but the loop must stay live so a later retransmit — once the
  // socket recovers — delivers and acks.
  const unhandled = [];
  const onUnhandled = (reason) => unhandled.push(reason);
  process.on("unhandledRejection", onUnhandled);
  try {
    const h = harness({ failSends: 1 });
    await drainMicrotasks();
    assert.equal(h.sendCalls, 1, "first send fired and rejected");

    // A later tick, after the socket recovers, resends successfully.
    h.tick();
    await drainMicrotasks();
    assert.equal(h.sendCalls, 2, "the loop retries after a rejected send");

    h.push(frame());
    assert.equal(await h.outcome, "acked");
    assert.equal(h.unsubscribeCalls, 1);
    assert.equal(h.cancelRetransmitCalls, 1);
  } finally {
    process.off("unhandledRejection", onUnhandled);
  }
  assert.deepEqual(unhandled, [], "a rejected send must not surface unhandled");
});

test("retransmitPermissionDecision expires cleanly when every send rejects", async () => {
  const unhandled = [];
  const onUnhandled = (reason) => unhandled.push(reason);
  process.on("unhandledRejection", onUnhandled);
  try {
    // Every send rejects (permanent transport failure). The loop must not throw
    // — it keeps retrying until the deadline, then resolves "expired".
    const h = harness({ failSends: Infinity });
    await drainMicrotasks();
    assert.equal(h.sendCalls, 1, "first send fired and rejected");

    h.tick();
    await drainMicrotasks();
    assert.equal(h.sendCalls, 2, "keeps retrying through rejection");

    h.expire();
    h.tick();
    assert.equal(await h.outcome, "expired");
    assert.equal(h.sendCalls, 2, "no resend past the deadline");
    assert.equal(h.unsubscribeCalls, 1, "listener torn down at expiry");
    assert.equal(h.cancelRetransmitCalls, 1, "scheduler torn down at expiry");
  } finally {
    process.off("unhandledRejection", onUnhandled);
  }
  assert.deepEqual(unhandled, [], "rejected sends must not surface unhandled");
});

test("retransmitPermissionDecision: channel_full keeps the loop active and resends until acked", async () => {
  // `channel_full` is a transient queue-saturation signal. The loop must NOT
  // settle on it — it stays subscribed and the scheduler keeps firing. A later
  // `sent` frame (once the queue drains) must settle `acked`.
  const h = harness();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 1, "first send fired");

  // Harness replies with channel_full — loop must stay active.
  h.push(frame({ status: "channel_full" }));
  await drainMicrotasks();
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });
  await drainMicrotasks();
  assert.equal(settled, false, "channel_full must not settle the loop");
  assert.equal(
    h.cancelRetransmitCalls,
    0,
    "scheduler must still be running after channel_full",
  );
  assert.equal(
    h.unsubscribeCalls,
    0,
    "listener must still be subscribed after channel_full",
  );

  // Scheduler fires on the next tick — loop resends.
  h.tick();
  await drainMicrotasks();
  assert.equal(h.sendCalls, 2, "loop resends on next tick after channel_full");

  // A later `sent` frame settles the loop.
  h.push(frame({ status: "sent" }));
  assert.equal(await h.outcome, "acked");
  assert.equal(h.unsubscribeCalls, 1, "listener torn down on acked");
  assert.equal(h.cancelRetransmitCalls, 1, "scheduler torn down on acked");
});

test("retransmitPermissionDecision: channel_full then deadline expires without acking resolves expired", async () => {
  // If the deadline fires while waiting for the queue to drain, the loop
  // resolves "expired" (fail-closed) — not "failed" and not stuck open.
  const h = harness();
  await drainMicrotasks();

  h.push(frame({ status: "channel_full" }));
  await drainMicrotasks();

  h.expire();
  h.tick();
  assert.equal(await h.outcome, "expired");
  assert.equal(
    h.sendCalls,
    1,
    "no resend after deadline while waiting on channel_full",
  );
});

test("retransmitPermissionDecision resolves failed on authoritative negative control_result statuses", async () => {
  // The three authoritative failure statuses (no_active_turn / channel_closed /
  // no_channel) mean the harness answered with a routing refusal. The loop must
  // stop retransmitting (re-sending cannot change the refusal) and resolve
  // "failed" so the card can re-enable for owner retry.
  for (const status of ["no_active_turn", "channel_closed", "no_channel"]) {
    const h = harness();
    await drainMicrotasks();
    assert.equal(h.sendCalls, 1, `first send fired (${status})`);

    h.push(frame({ status }));
    assert.equal(
      await h.outcome,
      "failed",
      `authoritative status "${status}" must resolve "failed"`,
    );
    assert.equal(h.unsubscribeCalls, 1, `listener torn down on "${status}"`);
    assert.equal(
      h.cancelRetransmitCalls,
      1,
      `scheduler torn down on "${status}"`,
    );

    // A tick after failure must NOT resend — the loop has settled.
    h.tick();
    await drainMicrotasks();
    assert.equal(
      h.sendCalls,
      1,
      `no resend after failure settle on "${status}"`,
    );
  }
});

test("retransmitPermissionDecision: negative result then retry delivers acked", async () => {
  // Carl's exact regression: a failure reply leaves the card actionable, the
  // owner retries (new harness() = fresh loop), and the second attempt succeeds.
  const first = harness();
  await drainMicrotasks();
  first.push(frame({ status: "no_active_turn" }));
  assert.equal(await first.outcome, "failed");

  // Owner retries — fresh orchestrator instance.
  const second = harness();
  await drainMicrotasks();
  second.push(frame({ status: "sent" }));
  assert.equal(await second.outcome, "acked");
  assert.equal(second.unsubscribeCalls, 1);
  assert.equal(second.cancelRetransmitCalls, 1);
});

test("retransmitPermissionDecision: frame after failure settlement is inert", async () => {
  // A late duplicate `control_result` for the same nonce arriving after the
  // loop has already settled (via a failure status) must not re-resolve.
  const h = harness();
  await drainMicrotasks();
  h.push(frame({ status: "no_active_turn" }));
  assert.equal(await h.outcome, "failed");

  // The unsubscribe detaches the listener, so a late push is dropped.
  // Verify the loop doesn't try to double-resolve or re-enable a second loop.
  h.push(frame({ status: "sent" })); // arrives after settle — inert
  h.tick(); // tick after settle must not resend
  await drainMicrotasks();
  assert.equal(h.sendCalls, 1, "no resend after settled failure");
  assert.equal(h.unsubscribeCalls, 1, "listener detached exactly once");
});
