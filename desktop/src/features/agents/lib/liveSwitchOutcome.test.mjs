import assert from "node:assert/strict";
import test from "node:test";

import {
  awaitLiveSwitchOutcome,
  createTerminalClaimJournal,
} from "./liveSwitchOutcome.ts";

const MODEL = "goose-claude-fable-5";
const CHANNEL_A = "channel-a";
const CHANNEL_B = "channel-b";
const REQUEST_ID = "a".repeat(32);
const REQUEST_BOUNDARY_MS = Date.parse("2026-07-28T12:00:00.000Z");
let frameId = 0;

function frame(status, overrides = {}) {
  frameId += 1;
  return {
    type: "switch_model",
    status,
    modelId: MODEL,
    channelId: CHANNEL_A,
    requestId: REQUEST_ID,
    relayEventId: frameId.toString(16).padStart(64, "0"),
    relayCreatedAt: Math.floor((REQUEST_BOUNDARY_MS + 1_000) / 1_000),
    observerTimestamp: new Date(REQUEST_BOUNDARY_MS + 1_000).toISOString(),
    observerSeq: frameId,
    ...overrides,
  };
}

/**
 * A controllable test harness mirroring the real wiring: a single-listener
 * pub/sub whose unsubscribe genuinely detaches (so post-unsubscribe pushes are
 * no-ops, matching `observerRelayStore`), a manual timeout, and a deferred
 * `sendSwitches` the test resolves explicitly.
 */
function harness(channelIds, requestId = REQUEST_ID) {
  let listener = null;
  let timeoutCb = null;
  let unsubscribeCalls = 0;
  let cancelTimeoutCalls = 0;
  let sendResolve;
  let sentRequestId = null;
  let subscribedWhenSent = false;
  const sendStarted = new Promise((resolve) => {
    sendResolve = resolve;
  });

  const outcome = awaitLiveSwitchOutcome({
    channelIds,
    modelId: MODEL,
    createRequestId: () => requestId,
    subscribe: (fn) => {
      listener = fn;
      return () => {
        unsubscribeCalls += 1;
        listener = null;
      };
    },
    sendSwitches: (actualRequestId) => {
      sentRequestId = actualRequestId;
      subscribedWhenSent = listener !== null;
      sendResolve();
      return Promise.resolve();
    },
    scheduleTimeout: (cb) => {
      timeoutCb = cb;
      return () => {
        cancelTimeoutCalls += 1;
      };
    },
    now: () => REQUEST_BOUNDARY_MS,
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
    get sentRequestId() {
      return sentRequestId;
    },
    get subscribedWhenSent() {
      return subscribedWhenSent;
    },
  };
}

test("awaitLiveSwitchOutcome fast sent on one channel does not mask a later unsupported on another", async () => {
  const h = harness([CHANNEL_A, CHANNEL_B]);
  // Channel A only acknowledges receipt. The fail-fast contract must keep
  // waiting and then reject when B proves the model unsupported.
  h.push(frame("sent"));
  h.push(frame("unsupported_model", { channelId: CHANNEL_B }));
  assert.equal(await h.outcome, "unsupported");
});

test("awaitLiveSwitchOutcome resolves ok only after every channel proves application", async () => {
  const h = harness([CHANNEL_A, CHANNEL_B]);
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

  h.push(frame("sent", { channelId: CHANNEL_A }));
  await drainMicrotasks();
  assert.equal(settled, false, "receipt is not proof of application");

  h.push(frame("recycling", { channelId: CHANNEL_B }));
  await drainMicrotasks();
  assert.equal(
    settled,
    false,
    "replacement scheduling is not proof of application",
  );

  h.push(frame("switched", { channelId: CHANNEL_A }));
  await drainMicrotasks();
  assert.equal(settled, false, "must wait for every target channel");

  h.push(frame("switched", { channelId: CHANNEL_B }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome rejects on unsupported immediately and unsubscribes exactly once", async () => {
  const h = harness([CHANNEL_A, CHANNEL_B]);
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
  const h = harness([CHANNEL_A]);
  h.push(frame("sent", { modelId: "some-other-model" }));
  h.push({ type: "cancel_turn", status: "sent", modelId: MODEL });
  h.push(frame("switched", { channelId: "unrelated-channel" }));
  h.push(frame("switched", { requestId: "b".repeat(32) }));
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });
  await Promise.resolve();
  assert.equal(settled, false, "unrelated frames must not advance the count");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome stays pending when the harness never proves application", async () => {
  const h = harness([CHANNEL_A, CHANNEL_B]);
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
  assert.equal(h.unsubscribeCalls, 1, "timeout fallback unsubscribes");
});

test("awaitLiveSwitchOutcome timeout is not blocked by a hung relay send", async () => {
  let fireTimeout = () => {};
  const outcome = awaitLiveSwitchOutcome({
    channelIds: [CHANNEL_A],
    modelId: MODEL,
    createRequestId: () => REQUEST_ID,
    subscribe: () => () => {},
    sendSwitches: () => new Promise(() => {}),
    scheduleTimeout: (onTimeout) => {
      fireTimeout = onTimeout;
      return () => {};
    },
    now: () => REQUEST_BOUNDARY_MS,
  });

  fireTimeout();
  const result = await Promise.race([
    outcome,
    new Promise((resolve) => setImmediate(() => resolve("still-blocked"))),
  ]);
  assert.equal(result, "pending");
});

for (const [status, expected] of [
  ["unsupported_model", "unsupported"],
  ["switch_failed", "failed"],
  ["switched", "ok"],
]) {
  test(`awaitLiveSwitchOutcome ${expected} proof is not blocked by a hung relay send`, async () => {
    let listener = () => {};
    const outcome = awaitLiveSwitchOutcome({
      channelIds: [CHANNEL_A],
      modelId: MODEL,
      createRequestId: () => REQUEST_ID,
      subscribe: (onFrame) => {
        listener = onFrame;
        return () => {};
      },
      sendSwitches: () => new Promise(() => {}),
      scheduleTimeout: () => () => {},
      now: () => REQUEST_BOUNDARY_MS,
    });

    listener(frame(status));
    const result = await Promise.race([
      outcome,
      new Promise((resolve) => setImmediate(() => resolve("still-blocked"))),
    ]);
    assert.equal(result, expected);
  });
}

test("awaitLiveSwitchOutcome rejects when the relay send fails before proof", async () => {
  const outcome = awaitLiveSwitchOutcome({
    channelIds: [CHANNEL_A],
    modelId: MODEL,
    createRequestId: () => REQUEST_ID,
    subscribe: () => () => {},
    sendSwitches: () => Promise.reject(new Error("relay send failed")),
    scheduleTimeout: () => () => {},
    now: () => REQUEST_BOUNDARY_MS,
  });

  await assert.rejects(outcome, /relay send failed/);
});

test("awaitLiveSwitchOutcome observes a relay send rejection after proof", async () => {
  let listener = () => {};
  let rejectSend = () => {};
  let unhandled;
  const onUnhandled = (reason) => {
    unhandled = reason;
  };
  process.on("unhandledRejection", onUnhandled);
  try {
    const outcome = awaitLiveSwitchOutcome({
      channelIds: [CHANNEL_A],
      modelId: MODEL,
      createRequestId: () => REQUEST_ID,
      subscribe: (onFrame) => {
        listener = onFrame;
        return () => {};
      },
      sendSwitches: () =>
        new Promise((_resolve, reject) => {
          rejectSend = reject;
        }),
      scheduleTimeout: () => () => {},
      now: () => REQUEST_BOUNDARY_MS,
    });

    listener(frame("switched"));
    assert.equal(await outcome, "ok");
    rejectSend(new Error("late relay send failure"));
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(unhandled, undefined);
  } finally {
    process.off("unhandledRejection", onUnhandled);
  }
});

test("awaitLiveSwitchOutcome fires the per-channel sends after subscribing", async () => {
  const h = harness([CHANNEL_A]);
  // The subscription is registered before the sends fire, so a frame arriving
  // mid-send is never dropped. Awaiting sendStarted proves sends ran.
  await h.sendStarted;
  assert.equal(h.sentRequestId, REQUEST_ID);
  assert.equal(h.subscribedWhenSent, true);
  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome with zero channels resolves ok without waiting for proof", async () => {
  const h = harness([]);
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome does not double-count repeated terminal frames", async () => {
  const h = harness([CHANNEL_A, CHANNEL_B]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("switched", { channelId: CHANNEL_A }));
  h.push(frame("switched", { channelId: CHANNEL_A }));
  await Promise.resolve();
  assert.equal(
    settled,
    false,
    "one channel cannot satisfy two target channels",
  );

  h.push(frame("switched", { channelId: CHANNEL_B }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome accepts exact-request proof emitted 1ms before the desktop clock", async () => {
  const h = harness([CHANNEL_A]);
  h.push(
    frame("switched", {
      relayCreatedAt: Math.floor(REQUEST_BOUNDARY_MS / 1_000),
      observerTimestamp: new Date(REQUEST_BOUNDARY_MS - 1).toISOString(),
    }),
  );
  h.fireTimeout();
  assert.equal(
    await h.outcome,
    "ok",
    "exact unpredictable request correlation is causal proof despite small cross-node skew",
  );
});

test("awaitLiveSwitchOutcome rejects a relay event outside the broad freshness window", async () => {
  const h = harness([CHANNEL_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(
    frame("switched", {
      relayCreatedAt: Math.floor((REQUEST_BOUNDARY_MS - 10 * 60_000) / 1_000),
    }),
  );
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
  assert.equal(
    settled,
    false,
    "an old signed relay envelope is stale even with an exact request ID",
  );

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome rejects observer payload time outside the broad freshness window", async () => {
  const h = harness([CHANNEL_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(
    frame("switched", {
      observerTimestamp: new Date(
        REQUEST_BOUNDARY_MS - 10 * 60_000,
      ).toISOString(),
    }),
  );
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
  assert.equal(
    settled,
    false,
    "a fresh relay envelope cannot make an old observer payload current",
  );

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome rejects relay and observer times beyond cross-node future skew", async () => {
  for (const overrides of [
    {
      relayCreatedAt: Math.floor((REQUEST_BOUNDARY_MS + 10 * 60_000) / 1_000),
    },
    {
      observerTimestamp: new Date(
        REQUEST_BOUNDARY_MS + 10 * 60_000,
      ).toISOString(),
    },
  ]) {
    const h = harness([CHANNEL_A]);
    let settled = false;
    void h.outcome.then(() => {
      settled = true;
    });
    h.push(frame("switched", overrides));
    for (let i = 0; i < 5; i++) {
      await Promise.resolve();
    }
    assert.equal(settled, false, "implausibly future proof must be ignored");
    h.push(frame("switched"));
    assert.equal(await h.outcome, "ok");
  }
});

test("awaitLiveSwitchOutcome fails closed when signed freshness metadata is absent", async () => {
  const h = harness([CHANNEL_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(
    frame("switched", {
      relayEventId: undefined,
      relayCreatedAt: undefined,
      observerTimestamp: undefined,
    }),
  );
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
  assert.equal(settled, false);

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("concurrent same-model requests resolve only their exact request result", async () => {
  const listeners = new Set();
  const startRequest = (requestId) =>
    awaitLiveSwitchOutcome({
      channelIds: [CHANNEL_A],
      modelId: MODEL,
      createRequestId: () => requestId,
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      sendSwitches: (sentRequestId) => {
        assert.equal(sentRequestId, requestId);
        return Promise.resolve();
      },
      scheduleTimeout: () => () => {},
      now: () => REQUEST_BOUNDARY_MS,
    });

  const requestA = "1".repeat(32);
  const requestB = "2".repeat(32);
  const first = startRequest(requestA);
  const second = startRequest(requestB);

  const secondFailure = frame("turn_ending", { requestId: requestB });
  for (const listener of [...listeners]) {
    listener(secondFailure);
  }
  const firstProof = frame("switched", { requestId: requestA });
  for (const listener of [...listeners]) {
    listener(firstProof);
  }

  assert.equal(await first, "ok");
  assert.equal(await second, "failed");
});

test("more than 4096 sequential completed operations remain live", async () => {
  for (let index = 0; index < 4_100; index += 1) {
    const requestId = index.toString(16).padStart(32, "0");
    const h = harness([CHANNEL_A], requestId);
    h.push(
      frame("switched", {
        requestId,
        relayEventId: (index + 10_000).toString(16).padStart(64, "0"),
      }),
    );
    h.fireTimeout();
    assert.equal(
      await h.outcome,
      "ok",
      `completed operation ${index} must not be disabled by a global claim cap`,
    );
  }
});

test("a duplicate relay ID and request pair cannot satisfy another request", async () => {
  const requestId = "c".repeat(32);
  const relayEventId = "d".repeat(64);
  const first = harness([CHANNEL_A], requestId);
  first.push(frame("switched", { requestId, relayEventId }));
  assert.equal(await first.outcome, "ok");

  const second = harness([CHANNEL_A], requestId);
  second.push(frame("switched", { requestId, relayEventId }));
  second.fireTimeout();
  assert.equal(await second.outcome, "pending");
});

test("terminal claims expire after their whole freshness horizon", () => {
  let currentTimeMs = REQUEST_BOUNDARY_MS;
  const claim = createTerminalClaimJournal({
    retentionMs: 1_000,
    now: () => currentTimeMs,
  });
  const proof = frame("switched", {
    requestId: "e".repeat(32),
    relayEventId: "f".repeat(64),
  });

  assert.equal(claim(proof), true);
  assert.equal(claim(proof), false);
  currentTimeMs += 1_001;
  assert.equal(
    claim(proof),
    true,
    "expired claims must be reclaimed so the journal is freshness-bounded",
  );
});

test("the production request ID generator does not reuse an operation ID", async () => {
  const requestIds = [];
  for (let index = 0; index < 2; index += 1) {
    let listener = null;
    const outcome = awaitLiveSwitchOutcome({
      channelIds: [CHANNEL_A],
      modelId: MODEL,
      subscribe: (nextListener) => {
        listener = nextListener;
        return () => {
          listener = null;
        };
      },
      sendSwitches: (requestId) => {
        requestIds.push(requestId);
        listener?.(
          frame("switched", {
            requestId,
            relayEventId: (index + 20_000).toString(16).padStart(64, "0"),
          }),
        );
        return Promise.resolve();
      },
      scheduleTimeout: () => () => {},
      now: () => REQUEST_BOUNDARY_MS,
    });
    assert.equal(await outcome, "ok");
  }

  assert.match(requestIds[0], /^[0-9a-f]{32}$/);
  assert.match(requestIds[1], /^[0-9a-f]{32}$/);
  assert.notEqual(requestIds[0], requestIds[1]);
});

for (const status of ["switch_failed", "turn_ending", "no_active_turn"]) {
  test(`awaitLiveSwitchOutcome reports ${status} as a failed application`, async () => {
    const h = harness([CHANNEL_A]);
    h.push(frame(status));
    assert.equal(await h.outcome, "failed");
  });
}
