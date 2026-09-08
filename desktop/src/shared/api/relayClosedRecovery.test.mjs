import assert from "node:assert/strict";
import test from "node:test";

import {
  handleRelayClosed,
  handleSubscriptionEose,
} from "./relayClosedRecovery.ts";
import {
  requestFirstEventGated,
  requestHistoryGated,
} from "./relayGateBoundary.ts";

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

test("production CLOSED handler rejects non-rate-limited history immediately and clears its timeout", () => {
  const originalWindow = globalThis.window;
  const clearedTimeouts = [];
  globalThis.window = {
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
          filter: { kinds: [0], authors: ["pubkey-1"], limit: 10 },
          events: [],
          resolve: () => assert.fail("CLOSED must not resolve history"),
          reject: (error) => errors.push(error),
          timeout: 42,
          timeoutMs: 25_000,
        },
      ],
    ]);
    const input = {
      subscriptions,
      subId: "history-1",
      sendReq: () => Promise.resolve(),
    };
    // Non-rate-limited CLOSED should reject immediately.
    handleRelayClosed({ ...input, message: "error: database unavailable" });
    handleRelayClosed({ ...input, message: "late CLOSED" });
    assert.equal(subscriptions.has("history-1"), false);
    assert.deepEqual(clearedTimeouts, [42]);
    assert.equal(errors.length, 1);
    assert.equal(errors[0].message, "error: database unavailable");
  } finally {
    globalThis.window = originalWindow;
  }
});

test("rate-limited history CLOSED schedules retry instead of rejecting immediately", () => {
  resetAll(0);
  const errors = [];
  const reqsSent = [];
  const subscriptions = new Map([
    [
      "history-rl",
      {
        mode: "history",
        filter: { kinds: [0], authors: ["pubkey-1"], limit: 10 },
        events: [],
        resolve: () => assert.fail("must not resolve before retry"),
        reject: (error) => errors.push(error),
        timeout: 99,
        timeoutMs: 25_000,
      },
    ],
  ]);
  handleRelayClosed({
    subscriptions,
    subId: "history-rl",
    message: "rate-limited: quota exceeded; retry in 5s",
    sendReq: (id, filter) => {
      reqsSent.push({ id, filter });
      return Promise.resolve();
    },
  });
  // Subscription should NOT have been rejected immediately.
  assert.equal(
    errors.length,
    0,
    "must not reject immediately on first attempt",
  );
  // Original subId evicted, a new one registered.
  assert.equal(subscriptions.has("history-rl"), false);
  // No retry fired yet at t=0.
  assert.equal(reqsSent.length, 0, "retry must not fire before delay");
  // Fire the retry timer (hint=5s → delayMs=5000).
  tickTo(5_001);
  assert.equal(
    reqsSent.length,
    1,
    "retry REQ must fire after rate-limit window",
  );
});

test("rate-limited history CLOSED exhausts 3 retries then rejects permanently", () => {
  resetAll(0);
  const errors = [];
  const reqsSent = [];
  const filter = { kinds: [0], authors: ["pk"], limit: 1 };
  const subscription = {
    mode: "history",
    filter,
    events: [],
    resolve: () => assert.fail("must not resolve"),
    reject: (error) => errors.push(error),
    timeout: 0,
    timeoutMs: 25_000,
  };
  const subscriptions = new Map([["history-rl", subscription]]);
  const sendReq = (id, _filter) => {
    reqsSent.push(id);
    return Promise.resolve();
  };

  // Simulate 3 rate-limited CLOSEDs (exhausts the attempt budget).
  for (let i = 0; i < 3; i++) {
    // The current live subId rotates on each retry; find it.
    const currentSubId = [...subscriptions.keys()].find((k) =>
      k.startsWith("history"),
    );
    assert.ok(currentSubId, `must have a live history sub on attempt ${i}`);
    handleRelayClosed({
      subscriptions,
      subId: currentSubId,
      message: "rate-limited: quota exceeded; retry in 1s",
      sendReq,
    });
    assert.equal(
      errors.length,
      0,
      `must not reject before attempt ${i + 1} fires`,
    );
    tickTo((i + 1) * 1_001);
    assert.equal(reqsSent.length, i + 1, `sendReq call ${i + 1} must fire`);
  }

  // 4th CLOSED — attempt budget exhausted → permanent reject.
  const currentSubId = [...subscriptions.keys()].find((k) =>
    k.startsWith("history"),
  );
  assert.ok(
    currentSubId,
    "must still have a live history sub before last CLOSED",
  );
  handleRelayClosed({
    subscriptions,
    subId: currentSubId,
    message: "rate-limited: quota exceeded; retry in 1s",
    sendReq,
  });
  assert.equal(errors.length, 1, "must reject after 3 failed retries");
  assert.equal(subscriptions.size, 0, "must evict sub after permanent reject");
});

test("rate-limited history retry op-timeout sends CLOSE and rejects without leaking the sub", () => {
  // Regression for: retry-path op-timeout deleted the sub and rejected but
  // never sent CLOSE for the rotated subId — leaving the relay holding a slot
  // against its per-connection cap.
  //
  // Also validates the late-EOSE non-regression: once the sub is evicted and
  // CLOSE sent, a late EOSE cannot double-resolve the promise because
  // handleSubscriptionEose guards on subscriptions.has(subId).
  resetAll(0);
  const errors = [];
  const closedSubIds = [];
  const filter = { kinds: [9], "#h": ["ch-close"], limit: 50 };
  const subscription = {
    mode: "history",
    filter,
    events: [],
    resolve: () => assert.fail("must not resolve"),
    reject: (error) => errors.push(error),
    timeout: 0,
    timeoutMs: 5_000,
  };
  const subscriptions = new Map([["history-close", subscription]]);

  // Trigger a rate-limited CLOSED — subscription rotates to a new subId.
  handleRelayClosed({
    subscriptions,
    subId: "history-close",
    message: "rate-limited: quota exceeded; retry in 1s",
    sendReq: () => Promise.resolve(),
    closeSubscription: async (id) => {
      closedSubIds.push(id);
    },
  });
  assert.equal(errors.length, 0, "must not reject before delay fires");

  // Find the rotated subId.
  const newSubId = [...subscriptions.keys()].find((k) =>
    k.startsWith("history"),
  );
  assert.ok(newSubId, "rotated sub must be registered");
  assert.notEqual(newSubId, "history-close", "sub must have been rotated");

  // The retry-delay timer fires: sendReq runs and arms a new op-timeout.
  tickTo(1_001);
  assert.equal(errors.length, 0, "still must not reject immediately after REQ");

  // The op-timeout fires (delay + timeoutMs = 1001 + 5000 = 6001 ms).
  tickTo(6_002);
  assert.equal(
    errors.length,
    1,
    "must reject after op-timeout on the retried sub",
  );
  assert.ok(
    closedSubIds.includes(newSubId),
    `CLOSE must be sent for the rotated sub ${newSubId} (got: ${JSON.stringify(closedSubIds)})`,
  );
  assert.equal(
    subscriptions.has(newSubId),
    false,
    "rotated sub must be evicted after op-timeout",
  );

  // Late-EOSE non-regression: arriving after eviction must be a no-op.
  const prevErrors = errors.length;
  handleSubscriptionEose({
    subscriptions,
    subId: newSubId,
    closeSubscription: async () => {},
  });
  assert.equal(
    errors.length,
    prevErrors,
    "late EOSE after eviction must not alter error state",
  );
});

test("rate-limited history CLOSED arms the shared gate for concurrent ops", () => {
  resetAll(0);
  const subscriptions = new Map([
    [
      "history-1",
      {
        mode: "history",
        filter: { kinds: [0], authors: ["pubkey-1"], limit: 10 },
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
        timeoutMs: 25_000,
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
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
        timeoutMs: 25_000,
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
        filter: { kinds: [9], "#h": ["ch-gate"], limit: 50 },
        events: [],
        resolve: () => {},
        reject: () => {},
        timeout: 0,
        timeoutMs: 25_000,
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

test("first-event request resolves null when EOSE arrives without an event", async () => {
  resetAll(0);
  const subscriptions = new Map();
  let requestedSubId = "";
  const firstEventPromise = requestFirstEventGated(
    subscriptions,
    async (payload) => {
      requestedSubId = payload[1];
    },
    async () => {},
    { kinds: [13_534], limit: 1 },
    25_000,
  );

  await Promise.resolve();
  assert.match(requestedSubId, /^first-/);

  handleSubscriptionEose({
    subscriptions,
    subId: requestedSubId,
    closeSubscription: async () => {},
  });

  assert.equal(await firstEventPromise, null);
  assert.equal(subscriptions.has(requestedSubId), false);
});

test("live readiness distinguishes EOSE from CLOSED", () => {
  const readiness = [];
  const subscriptions = new Map([
    [
      "live-eose",
      {
        mode: "live",
        filter: { kinds: [9], limit: 0 },
        onEvent: () => {},
        resolveReady: (result) => readiness.push(result),
      },
    ],
    [
      "live-closed",
      {
        mode: "live",
        filter: { kinds: [9], limit: 0 },
        onEvent: () => {},
        resolveReady: (result) => readiness.push(result),
      },
    ],
  ]);

  handleSubscriptionEose({
    subscriptions,
    subId: "live-eose",
    closeSubscription: async () => {},
  });
  handleRelayClosed({
    subscriptions,
    subId: "live-closed",
    message: "restricted: access revoked",
    sendReq: () => Promise.resolve(),
  });

  assert.deepEqual(readiness, ["eose", "closed"]);
});

test("production CLOSED handler removes terminal live subscriptions", () => {
  let readyCalls = 0;
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

test("rate-limited CLOSED retry delay is max(backoff, gate remaining), not just hint", () => {
  resetAll(0);
  // Activate a long gate first (20s), then send a shorter-hint CLOSED (5s).
  // The retry delay must use the gate remaining time (20s), not the hint (5s).
  activateRateLimit(20); // gate expires at 20_000 ms

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

  // Should fire at 20s.
  tickTo(20_001);
  assert.equal(firedAt.length, 1, "retry must fire after gate remaining time");
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

// ── Test: rejecting closeSubscription does not produce an unhandled rejection ─
//
// Load-bearing for the `.catch(() => {})` guard on the op-timeout CLOSE send.
//
// When the IPC/socket call inside closeSubscription throws (e.g. the connection
// was already torn down), the rejection must be swallowed — the history promise
// is already being rejected by the line below the CLOSE send. Without the catch,
// a Node/browser unhandled-rejection event fires and surfaces as a test failure.

test("rejecting closeSubscription on op-timeout does not produce an unhandled rejection", async () => {
  resetAll(0);
  const errors = [];
  const filter = { kinds: [9], "#h": ["ch-reject"], limit: 50 };
  const subscription = {
    mode: "history",
    filter,
    events: [],
    resolve: () => assert.fail("must not resolve"),
    reject: (error) => errors.push(error),
    timeout: 0,
    timeoutMs: 5_000,
  };
  const subscriptions = new Map([["history-reject", subscription]]);

  // closeSubscription that rejects — simulates a broken socket.
  handleRelayClosed({
    subscriptions,
    subId: "history-reject",
    message: "rate-limited: quota exceeded; retry in 1s",
    sendReq: () => Promise.resolve(),
    closeSubscription: async () => {
      throw new Error("socket already closed");
    },
  });

  // Fire the retry delay and op-timeout.
  tickTo(1_001);
  tickTo(6_002);

  // Let any async promise chains settle.
  await Promise.resolve();
  await Promise.resolve();

  // The history promise must have been rejected exactly once despite the
  // closeSubscription rejection — no unhandled rejection from the CLOSE send.
  assert.equal(
    errors.length,
    1,
    "history promise must be rejected exactly once; the CLOSE rejection is swallowed",
  );
  assert.ok(
    errors[0].message.includes("Relay closed"),
    `rejection must be the history error, not the CLOSE error (got: ${errors[0].message})`,
  );
});

// ── Test: CLOSE wiring at the production call site ────────────────────────────
//
// Falsifiable guard: asserts that relayClientSession.ts passes closeSubscription
// to handleRelayClosed. Removing the wiring line makes this test promptly red —
// closing the gap where the helper behavioral test passes without the actual
// production CLOSE being sent.

test("relayClientSession wires closeSubscription into handleRelayClosed", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(
    new URL("./relayClientSession.ts", import.meta.url),
    "utf8",
  );
  // The handleRelayClosed call must pass a closeSubscription callback.
  assert.match(
    source,
    /handleRelayClosed\(\{[^}]*closeSubscription[^}]*\}\)/,
    "relayClientSession.ts must pass closeSubscription to handleRelayClosed",
  );
  // The callback must reach this.closeSubscription so it sends a real CLOSE.
  assert.match(
    source,
    /closeSubscription:\s*\([^)]+\)\s*=>\s*this\.closeSubscription\(/,
    "closeSubscription callback must delegate to this.closeSubscription()",
  );
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
