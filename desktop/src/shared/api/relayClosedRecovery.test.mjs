import assert from "node:assert/strict";
import test from "node:test";

import {
  handleRelayClosed,
  handleSubscriptionEose,
  releaseLiveSubscription,
} from "./relayClosedRecovery.ts";
import { requestHistoryGated } from "./relayGateBoundary.ts";
import { replayLiveSubscriptions } from "./relayReconnectReplay.ts";

// ── Fake-timer setup ──────────────────────────────────────────────────────────
// The rate-limit gate and closed-retry logic use window.setTimeout/clearTimeout.

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;

function fakeSetTimeout(fn, ms) {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
  return id;
}

function fakeClearTimeout(id) {
  pendingTimers.delete(id);
}

function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
}

const origDateNow = Date.now;
function setFakeNow(ms) {
  fakeNow = ms;
  Date.now = () => fakeNow;
}

globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
};

// Import gate after window shim is installed.
const { activateRateLimit, isRateLimited, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

function resetAll(startMs = 0) {
  pendingTimers.clear();
  nextTimerId = 1;
  setFakeNow(startMs);
  resetRateLimitGate();
}

test("production CLOSED handler rejects history once and clears its timeout", () => {
  const originalWindow = globalThis.window;
  const clearedTimeouts = [];
  globalThis.window = {
    // Provide both setTimeout (needed by activateRateLimit in the F1 fix) and
    // clearTimeout (the existing assertion target).
    setTimeout: (_fn, _ms) => 0,
    clearTimeout: (timeout) => clearedTimeouts.push(timeout),
  };
  try {
    const errors = [];
    const subscriptions = new Map([
      [
        "history-1",
        {
          mode: "history",
          events: [],
          resolve: () => assert.fail("CLOSED must not resolve history"),
          reject: (error) => errors.push(error),
          timeout: 42,
        },
      ],
    ]);
    const input = {
      subscriptions,
      subId: "history-1",
      sendReq: () => Promise.resolve(),
    };
    handleRelayClosed({
      ...input,
      message: "rate-limited: too many concurrent requests",
    });
    handleRelayClosed({ ...input, message: "late CLOSED" });
    assert.equal(subscriptions.has("history-1"), false);
    assert.deepEqual(clearedTimeouts, [42]);
    assert.equal(errors.length, 1);
    assert.equal(
      errors[0].message,
      "rate-limited: too many concurrent requests",
    );
  } finally {
    globalThis.window = originalWindow;
  }
});

test("rate-limited history CLOSED arms the shared gate for concurrent ops", () => {
  resetAll(0);
  const subscriptions = new Map([
    [
      "history-1",
      {
        mode: "history",
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "history-1",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: () => Promise.resolve(),
  });
  assert.equal(
    isRateLimited(),
    true,
    "gate must be active after rate-limited history CLOSED",
  );
  // Gate expires after the hint duration.
  tickTo(5_001);
  assert.equal(isRateLimited(), false, "gate must clear after hint duration");
});

test("non-rate-limited history CLOSED does not arm the gate", () => {
  resetAll(0);
  const subscriptions = new Map([
    [
      "history-2",
      {
        mode: "history",
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "history-2",
    message: "error: database unavailable",
    sendReq: () => Promise.resolve(),
  });
  assert.equal(
    isRateLimited(),
    false,
    "gate must remain inactive for non-rate-limited history CLOSED",
  );
});

test("gate armed by rate-limited history CLOSED defers the next REQ until expiry then resumes", async () => {
  // Simulate: rate-limited CLOSED arrives on a history sub → gate arms for 5s.
  // A concurrent requestHistoryGated call must not issue the REQ before 5s,
  // and must issue it (and resolve) once the gate clears.
  resetAll(0);

  const subscriptions = new Map([
    [
      "history-gate",
      {
        mode: "history",
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
      },
    ],
  ]);

  // Arm the gate via a rate-limited history CLOSED.
  handleRelayClosed({
    subscriptions,
    subId: "history-gate",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: () => Promise.resolve(),
  });

  assert.equal(isRateLimited(), true, "gate must be armed before the test");

  const sentAt = [];
  const reqSubscriptions = new Map();

  // requestHistoryGated will await the gate, so the REQ must not fire at t=0.
  const historyPromise = requestHistoryGated(
    reqSubscriptions,
    async (payload) => {
      // Record when the REQ fires. The test harness sets up the EOSE path by
      // adding a history subscription to reqSubscriptions immediately after
      // the REQ is recorded, then resolving it.
      sentAt.push(fakeNow);
      // Resolve the returned promise by completing the sub synchronously.
      const subId = payload[1];
      const sub = reqSubscriptions.get(subId);
      if (sub) {
        window.clearTimeout(sub.timeout);
        reqSubscriptions.delete(subId);
        sub.resolve([]);
      }
    },
    async () => {},
    { kinds: [9], "#h": ["ch-test"], limit: 50 },
    25_000,
  );

  // REQ must not have fired yet — gate is still active at t=0.
  await Promise.resolve();
  assert.equal(sentAt.length, 0, "REQ must not fire while gate is active");

  // Expire the gate — the deferred REQ should fire.
  tickTo(5_001);

  await historyPromise;

  assert.equal(
    sentAt.length,
    1,
    "REQ must fire exactly once after gate clears",
  );
  assert.ok(sentAt[0] >= 5_001, "REQ must fire only after gate expiry");
});

test("production CLOSED handler removes terminal live subscriptions", () => {
  let readyCalls = 0;
  let reconnectEoseCalls = 0;
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], limit: 50 },
        onEvent: () => {},
        resolveReady: () => {
          readyCalls += 1;
        },
        resolveReconnectEose: () => {
          reconnectEoseCalls += 1;
        },
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "restricted: access revoked",
    sendReq: () => Promise.resolve(),
  });
  assert.equal(subscriptions.has("live-1"), false);
  assert.equal(readyCalls, 1);
  assert.equal(reconnectEoseCalls, 1);
});

test("retryable CLOSED preserves the original long-lived filter", async () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  globalThis.window = {
    clearTimeout: () => {},
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay });
      return 42;
    },
  };
  try {
    const sentFilters = [];
    const subscriptions = new Map([
      [
        "live-1",
        {
          mode: "live",
          filter: {
            kinds: [9],
            "#h": ["channel-1"],
            limit: 1_000,
            since: 900,
          },
          onEvent: () => {},
          lastSeenCreatedAt: 1_000,
        },
      ],
    ]);

    handleRelayClosed({
      subscriptions,
      subId: "live-1",
      message: "error: database unavailable",
      sendReq: async (_subId, filter) => {
        sentFilters.push(filter);
      },
    });

    assert.deepEqual(
      scheduled.map(({ delay }) => delay),
      [8_000, 1_000],
    );
    scheduled.find(({ delay }) => delay === 1_000).callback();
    await Promise.resolve();
    assert.deepEqual(sentFilters, [
      {
        kinds: [9],
        "#h": ["channel-1"],
        limit: 1_000,
        since: 900,
      },
    ]);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("retryable CLOSED recovery drains buffered events before completing", () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  globalThis.window = {
    clearTimeout: () => {},
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay });
      return scheduled.length;
    },
  };
  try {
    const lifecycle = [];
    const subscription = {
      mode: "live",
      filter: { kinds: [9], limit: 50 },
      onEvent: () => {},
      onClosedRecoveryStateChange: (recovering) =>
        lifecycle.push(recovering ? "recovering" : "live"),
    };
    const subscriptions = new Map([["live-1", subscription]]);
    const closedInput = {
      subscriptions,
      subId: "live-1",
      message: "error: database unavailable",
      sendReq: async () => {},
    };

    handleRelayClosed(closedInput);
    handleRelayClosed(closedInput);
    assert.deepEqual(lifecycle, ["recovering"]);
    assert.deepEqual(
      scheduled.map(({ delay }) => delay),
      [8_000, 1_000],
    );

    handleSubscriptionEose({
      subscriptions,
      subId: "live-1",
      closeSubscription: async () => {},
      beforeLiveRecoveryComplete: () => lifecycle.push("drained"),
    });
    assert.deepEqual(lifecycle, ["recovering", "drained", "live"]);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("repeated retryable CLOSED cannot extend the reconnect EOSE deadline", async () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  globalThis.window = {
    clearTimeout: () => {},
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay });
      return scheduled.length;
    },
  };
  try {
    const subscription = {
      mode: "live",
      filter: {
        kinds: [9],
        "#h": ["channel-1"],
        limit: 1_000,
        since: 900,
      },
      onEvent: () => {},
      lastSeenCreatedAt: 1_000,
    };
    const subscriptions = new Map([["live-1", subscription]]);
    const sentFilters = [];
    const replayOutcome = replayLiveSubscriptions({
      subscriptions,
      now: 2_000,
      eoseTimeoutMs: 10,
      sendRaw: async () => {},
      requestHistory: async () => [],
    }).then(
      () => "resolved",
      (error) => error,
    );

    await new Promise((resolve) => setImmediate(resolve));
    for (let attempt = 0; attempt < 3; attempt += 1) {
      handleRelayClosed({
        subscriptions,
        subId: "live-1",
        message: "error: database unavailable",
        sendReq: async (_subId, filter) => {
          sentFilters.push(filter);
        },
      });
      assert.equal(scheduled.filter(({ delay }) => delay === 8_000).length, 1);
      scheduled.find(({ delay }) => delay === 1_000 * 2 ** attempt).callback();
      await new Promise((resolve) => setImmediate(resolve));
    }

    const outcome = await replayOutcome;
    assert.equal(outcome instanceof Error, true);
    assert.match(outcome.message, /EOSE/);
    assert.equal(subscription.resolveReconnectEose, undefined);
    assert.deepEqual(
      sentFilters.map((filter) => filter.since),
      [900, 900, 900],
    );
    releaseLiveSubscription(subscription);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("standalone CLOSED recovery has a bounded EOSE deadline", async () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  const clearedTimeouts = [];
  globalThis.window = {
    clearTimeout: (timeout) => clearedTimeouts.push(timeout),
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay, id: scheduled.length + 1 });
      return scheduled.length;
    },
  };
  try {
    const lifecycle = [];
    let timeoutError;
    const subscription = {
      mode: "live",
      filter: { kinds: [9], limit: 50 },
      onEvent: () => {},
      onClosedRecoveryStateChange: (recovering) => lifecycle.push(recovering),
      onClosedRecoveryTimeout: (error) => {
        assert.equal(subscription.closedRecoveryInProgress, true);
        timeoutError = error;
        releaseLiveSubscription(subscription);
      },
    };
    const subscriptions = new Map([["live-1", subscription]]);

    handleRelayClosed({
      subscriptions,
      subId: "live-1",
      message: "error: database unavailable",
      sendReq: async () => {},
    });
    scheduled.find(({ delay }) => delay === 1_000).callback();
    await Promise.resolve();
    assert.deepEqual(lifecycle, [true]);

    scheduled.find(({ delay }) => delay === 8_000).callback();
    assert.match(timeoutError.message, /EOSE/);
    assert.deepEqual(lifecycle, [true, false]);
    assert.equal(subscription.closedRecoveryTimeout, undefined);
    assert.equal(clearedTimeouts.includes(1), false);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("socket reset starts CLOSED retry backoff from the base delay", () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  globalThis.window = {
    clearTimeout: () => {},
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay });
      return scheduled.length;
    },
  };
  try {
    const subscription = {
      mode: "live",
      filter: { kinds: [9], limit: 50 },
      onEvent: () => {},
      closedRetryAttempt: 4,
      onClosedRecoveryTimeout: () => releaseLiveSubscription(subscription),
    };
    const subscriptions = new Map([["live-1", subscription]]);
    const closedInput = {
      subscriptions,
      subId: "live-1",
      message: "error: database unavailable",
      sendReq: async () => {},
    };

    handleRelayClosed(closedInput);
    assert.deepEqual(
      scheduled.map(({ delay }) => delay),
      [8_000, 16_000],
    );
    scheduled.find(({ delay }) => delay === 8_000).callback();
    assert.equal(subscription.closedRetryAttempt, 0);

    handleRelayClosed(closedInput);
    assert.deepEqual(
      scheduled.slice(2).map(({ delay }) => delay),
      [8_000, 1_000],
    );
    assert.equal(subscription.closedRetryAttempt, 1);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("live EOSE releases the reconnect replay barrier", () => {
  let reconnectEoseCalls = 0;
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], limit: 50 },
        onEvent: () => {},
        resolveReconnectEose: () => {
          reconnectEoseCalls += 1;
        },
      },
    ],
  ]);

  handleSubscriptionEose({
    subscriptions,
    subId: "live-1",
    closeSubscription: () => Promise.resolve(),
  });

  assert.equal(reconnectEoseCalls, 1);
  assert.equal(subscriptions.get("live-1").resolveReconnectEose, undefined);
});

test("live subscription cleanup releases every reconnect waiter", () => {
  const originalWindow = globalThis.window;
  const clearedTimeouts = [];
  globalThis.window = {
    clearTimeout: (timeout) => clearedTimeouts.push(timeout),
  };
  try {
    let readyCalls = 0;
    let reconnectEoseCalls = 0;
    const subscription = {
      mode: "live",
      filter: { kinds: [9], limit: 50 },
      onEvent: () => {},
      resolveReady: () => {
        readyCalls += 1;
      },
      resolveReconnectEose: () => {
        reconnectEoseCalls += 1;
      },
      closedRetryTimeout: 42,
      closedRetryAttempt: 4,
      closedRecoveryInProgress: true,
      closedRecoveryTimeout: 43,
    };

    releaseLiveSubscription(subscription);

    assert.equal(readyCalls, 1);
    assert.equal(reconnectEoseCalls, 1);
    assert.equal(subscription.resolveReady, undefined);
    assert.equal(subscription.resolveReconnectEose, undefined);
    assert.equal(subscription.closedRetryTimeout, undefined);
    assert.equal(subscription.closedRetryAttempt, 0);
    assert.equal(subscription.closedRecoveryTimeout, undefined);
    assert.deepEqual(clearedTimeouts, [42, 43]);
  } finally {
    globalThis.window = originalWindow;
  }
});

// ── Rate-limited CLOSED core behaviour (F5) ───────────────────────────────────

test("rate-limited CLOSED keeps live subscription in the map", () => {
  resetAll(0);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        resolveReady: () => {},
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: () => Promise.resolve(),
  });
  assert.equal(
    subscriptions.has("live-1"),
    true,
    "subscription must survive rate-limited CLOSED",
  );
});

test("rate-limited CLOSED activates the rate-limit gate with the parsed hint", () => {
  resetAll(0);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        resolveReady: () => {},
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: () => Promise.resolve(),
  });
  assert.equal(
    isRateLimited(),
    true,
    "gate must be active after rate-limited CLOSED",
  );
  // Gate should expire at 5s.
  tickTo(5_001);
  assert.equal(isRateLimited(), false);
});

test("rate-limit delay stays gate-aware until the recovery deadline resets it", () => {
  resetAll(0);
  // Activate a long gate first (20s), then send a shorter-hint CLOSED (5s).
  // The retry is initially scheduled for the gate's remaining time (20s), but
  // the bounded 8s recovery deadline must release it first so a fresh socket
  // does not inherit a retry attempt that outlives its own replay deadline.
  activateRateLimit(20); // gate expires at 20_000 ms

  const firedAt = [];
  let recoveryTimeouts = 0;
  const subscription = {
    mode: "live",
    filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
    onEvent: () => {},
    resolveReady: () => {},
    onClosedRecoveryTimeout: () => {
      recoveryTimeouts += 1;
      releaseLiveSubscription(subscription);
    },
  };
  const subscriptions = new Map([["live-1", subscription]]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: () => {
      firedAt.push(fakeNow);
      return Promise.resolve();
    },
  });

  // Retry should NOT fire at 5s (hint) — the gate remaining is 20s.
  tickTo(5_001);
  assert.equal(
    firedAt.length,
    0,
    "retry must not fire before gate remaining time",
  );
  assert.equal(
    [...pendingTimers.values()].filter(({ fireAt }) => fireAt === 20_000)
      .length,
    2,
    "gate expiry and retry must both be scheduled at the longer gate boundary",
  );

  tickTo(8_001);
  assert.equal(recoveryTimeouts, 1, "recovery deadline must fire first");
  assert.equal(subscription.closedRetryAttempt, 0);

  // The old generation's retry was cleared by release and must not fire later.
  tickTo(20_001);
  assert.equal(firedAt.length, 0);
});

test("non-rate-limited retryable CLOSED still schedules a retry", () => {
  resetAll(0);
  const firedAt = [];
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        resolveReady: () => {},
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "error: database error",
    sendReq: () => {
      firedAt.push(fakeNow);
      return Promise.resolve();
    },
  });
  // Base delay is 1s for first attempt.
  tickTo(1_001);
  assert.equal(firedAt.length, 1, "retryable CLOSED must schedule a retry");
  assert.equal(
    subscriptions.has("live-1"),
    true,
    "subscription must survive retryable CLOSED",
  );
});

test("terminal CLOSED deletes subscription and does not retry", () => {
  resetAll(0);
  const firedAt = [];
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        resolveReady: () => {},
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "live-1",
    message: "restricted: not a member",
    sendReq: () => {
      firedAt.push(fakeNow);
      return Promise.resolve();
    },
  });
  assert.equal(
    subscriptions.has("live-1"),
    false,
    "terminal CLOSED must delete subscription",
  );
  tickTo(10_000);
  assert.equal(firedAt.length, 0, "terminal CLOSED must not retry");
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
