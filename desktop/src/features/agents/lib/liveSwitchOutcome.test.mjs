import assert from "node:assert/strict";
import test from "node:test";

import {
  awaitLiveSwitchOutcome,
  sendAllLiveSwitchRequests,
} from "./liveSwitchOutcome.ts";

const MODEL = "goose-claude-fable-5";

function frame(status, overrides = {}) {
  return { type: "switch_model", status, modelId: MODEL, ...overrides };
}

/**
 * A controllable test harness mirroring the real wiring: a single-listener
 * pub/sub whose unsubscribe genuinely detaches (so post-unsubscribe pushes are
 * no-ops, matching `observerRelayStore`), a manual timeout, and a deferred
 * `sendSwitches` the test resolves explicitly.
 */
function harness(channelCount, turnIds) {
  let listener = null;
  let timeoutCb = null;
  let unsubscribeCalls = 0;
  let cancelTimeoutCalls = 0;
  let sendResolve;
  const sendStarted = new Promise((resolve) => {
    sendResolve = resolve;
  });

  const outcome = awaitLiveSwitchOutcome({
    channelCount,
    modelId: MODEL,
    turnIds,
    subscribe: (fn) => {
      listener = fn;
      return () => {
        unsubscribeCalls += 1;
        listener = null;
      };
    },
    sendSwitches: () => {
      sendResolve();
      return Promise.resolve();
    },
    scheduleTimeout: (cb) => {
      timeoutCb = cb;
      return () => {
        cancelTimeoutCalls += 1;
      };
    },
  });

  return {
    outcome,
    sendStarted,
    push: (f) => listener?.(f),
    fireTimeout: () => timeoutCb?.(),
    get unsubscribeCalls() {
      return unsubscribeCalls;
    },
    get cancelTimeoutCalls() {
      return cancelTimeoutCalls;
    },
  };
}

test("awaitLiveSwitchOutcome fast sent on one channel does not mask a later unsupported on another", async () => {
  const h = harness(2);
  // Channel A acks fast as `sent`; a first-ack-resolves impl would settle "ok"
  // here. The fail-fast contract must keep waiting and then reject on B.
  h.push(frame("sent"));
  h.push(frame("unsupported_model"));
  assert.equal(await h.outcome, "unsupported");
});

test("awaitLiveSwitchOutcome resolves ok only after the last channel acks", async () => {
  const h = harness(3);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  // The `.then` that flips `settled` flushes on a later microtask tick than a
  // single drain, so a single `await Promise.resolve()` would let this
  // assertion pass even against a first-ack-resolves bug. Draining several
  // ticks guarantees a resolved promise's callback has run, so the interim
  // `settled === false` checks deterministically regress an early resolve.
  const drainMicrotasks = async () => {
    for (let i = 0; i < 5; i++) {
      await Promise.resolve();
    }
  };

  h.push(frame("sent"));
  await drainMicrotasks();
  assert.equal(settled, false, "delivery acknowledgement is not final success");

  h.push(frame("switched"));
  await drainMicrotasks();
  assert.equal(settled, false, "must not resolve after the first final result");

  h.push(frame("switched"));
  await drainMicrotasks();
  assert.equal(settled, false, "must not resolve before the last final result");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome rejects on unsupported immediately and unsubscribes exactly once", async () => {
  const h = harness(2);
  h.push(frame("unsupported_model"));
  assert.equal(await h.outcome, "unsupported");
  assert.equal(h.unsubscribeCalls, 1);
  assert.equal(h.cancelTimeoutCalls, 1);

  // A second rejection arriving after the first must not re-resolve or
  // re-unsubscribe — the listener is already detached.
  h.push(frame("unsupported_model"));
  assert.equal(h.unsubscribeCalls, 1, "no double-unsubscribe on a late frame");
});

test("awaitLiveSwitchOutcome ignores frames for a different model or control type", async () => {
  const h = harness(1);
  h.push(frame("sent", { modelId: "some-other-model" }));
  h.push({ type: "cancel_turn", status: "sent", modelId: MODEL });
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });
  await Promise.resolve();
  assert.equal(settled, false, "unrelated frames must not advance the count");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome correlates sibling acknowledgements by exact turn id", async () => {
  const h = harness(2, ["turn-a", "turn-b"]);
  h.push(frame("switched", { turnId: "turn-a" }));
  h.push(frame("switched", { turnId: "turn-a" }));
  h.push(frame("switched", { turnId: "stale-turn" }));

  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });
  await Promise.resolve();
  assert.equal(settled, false, "duplicate/stale turn acks must not over-count");

  h.push(frame("switched", { turnId: "turn-b" }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome fails closed when a targeted turn disappeared, is ending, application failed, or correlation is ambiguous", async () => {
  for (const status of [
    "no_active_turn",
    "turn_ending",
    "switch_failed",
    "ambiguous",
  ]) {
    const h = harness(1, ["turn-a"]);
    h.push(frame(status, { turnId: "turn-a" }));
    assert.equal(await h.outcome, "failed");
    assert.equal(h.unsubscribeCalls, 1);
    assert.equal(h.cancelTimeoutCalls, 1);
  }
});

test("awaitLiveSwitchOutcome reports failure after a sibling already switched", async () => {
  const h = harness(2, ["turn-a", "turn-b"]);
  h.push(frame("switched", { turnId: "turn-a" }));
  h.push(frame("switch_failed", { turnId: "turn-b" }));
  assert.equal(
    await h.outcome,
    "failed",
    "a mixed applied/rejected result must never be presented as atomic success",
  );
});

test("awaitLiveSwitchOutcome reports failed and cleans up when sending rejects", async () => {
  let listener = null;
  let unsubscribeCalls = 0;
  let cancelTimeoutCalls = 0;
  const failure = new Error("observer control send failed");

  const outcome = await awaitLiveSwitchOutcome({
    channelCount: 1,
    modelId: MODEL,
    turnIds: ["turn-a"],
    subscribe: (next) => {
      listener = next;
      return () => {
        unsubscribeCalls += 1;
        listener = null;
      };
    },
    sendSwitches: async () => {
      throw failure;
    },
    scheduleTimeout: () => () => {
      cancelTimeoutCalls += 1;
    },
  });

  assert.equal(outcome, "failed");
  assert.equal(listener, null);
  assert.equal(unsubscribeCalls, 1);
  assert.equal(cancelTimeoutCalls, 1);
});

test("sendAllLiveSwitchRequests attempts every sibling before reporting mixed delivery failure", async () => {
  const attempts = [];

  await assert.rejects(
    sendAllLiveSwitchRequests([
      async () => {
        attempts.push("turn-a");
      },
      async () => {
        attempts.push("turn-b");
        throw new Error("relay rejected turn-b");
      },
      async () => {
        attempts.push("turn-c");
      },
    ]),
    /Failed to deliver 1 of 3/,
  );

  assert.deepEqual(attempts, ["turn-a", "turn-b", "turn-c"]);
});

test("mixed live-send success and rejection resolves to the partial-application failure path", async () => {
  let listener = null;
  const attempted = [];
  const outcome = awaitLiveSwitchOutcome({
    channelCount: 2,
    modelId: MODEL,
    turnIds: ["turn-a", "turn-b"],
    subscribe: (next) => {
      listener = next;
      return () => {
        listener = null;
      };
    },
    sendSwitches: () =>
      sendAllLiveSwitchRequests([
        async () => {
          attempted.push("turn-a");
          listener?.(frame("switched", { turnId: "turn-a" }));
        },
        async () => {
          attempted.push("turn-b");
          throw new Error("publish failed");
        },
      ]),
    scheduleTimeout: () => () => {},
  });

  assert.equal(await outcome, "failed");
  assert.deepEqual(attempted, ["turn-a", "turn-b"]);
});

test("awaitLiveSwitchOutcome reports pending via the timeout fallback when the harness never replies", async () => {
  const h = harness(2);
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
  assert.equal(h.unsubscribeCalls, 1, "timeout fallback unsubscribes");
});

test("awaitLiveSwitchOutcome fires the per-channel sends after subscribing", async () => {
  const h = harness(1);
  // The subscription is registered before the sends fire, so a frame arriving
  // mid-send is never dropped. Awaiting sendStarted proves sends ran.
  await h.sendStarted;
  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome with zero channels reports pending at the timeout (no acks expected)", async () => {
  // No active turns means channelCount 0: remaining starts at 0 but the success
  // resolve only fires inside a frame callback, so with no frames the timeout
  // fallback is what settles it. This documents the degenerate path.
  const h = harness(0);
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
});
