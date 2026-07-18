import assert from "node:assert/strict";
import test from "node:test";

import {
  isRelayConnectionDegraded,
  rejectClosedSubscription,
  sortEvents,
} from "./relayClientShared.ts";

function event(id, createdAt) {
  return {
    id,
    pubkey: "pubkey",
    created_at: createdAt,
    kind: 9,
    tags: [],
    content: "",
    sig: "sig",
  };
}

test("sortEvents — same-second events sort by id, order-independent", () => {
  const a = event("aaa", 100);
  const b = event("bbb", 100);
  const c = event("ccc", 101);

  const forward = sortEvents([a, b, c]).map((e) => e.id);
  const shuffled = sortEvents([c, b, a]).map((e) => e.id);

  // Stable (created_at, id) order regardless of input order, matching the
  // cache sort (sortMessages) and the relay's id-ASC same-second tiebreak.
  assert.deepEqual(forward, ["aaa", "bbb", "ccc"]);
  assert.deepEqual(shuffled, ["aaa", "bbb", "ccc"]);
});

test("CLOSED rejects pending history once and clears its timeout", () => {
  const originalWindow = globalThis.window;
  const clearedTimeouts = [];
  globalThis.window = {
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

    assert.equal(
      rejectClosedSubscription(
        subscriptions,
        "history-1",
        "rate-limited: too many concurrent requests",
      ),
      true,
    );
    assert.equal(subscriptions.has("history-1"), false);
    assert.deepEqual(clearedTimeouts, [42]);
    assert.equal(errors.length, 1);
    assert.equal(
      errors[0].message,
      "rate-limited: too many concurrent requests",
    );

    assert.equal(
      rejectClosedSubscription(subscriptions, "history-1", "late CLOSED"),
      false,
    );
    assert.equal(errors.length, 1);
  } finally {
    globalThis.window = originalWindow;
  }
});

test("CLOSED settles live readiness and retains the subscription for replay", () => {
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

  assert.equal(
    rejectClosedSubscription(subscriptions, "live-1", "restricted"),
    true,
  );
  assert.equal(subscriptions.has("live-1"), true);
  assert.equal(readyCalls, 1);
  assert.equal(
    rejectClosedSubscription(subscriptions, "live-1", "late CLOSED"),
    true,
  );
  assert.equal(readyCalls, 1);
});

test("isRelayConnectionDegraded — healthy states are not degraded", () => {
  assert.equal(isRelayConnectionDegraded("idle"), false);
  assert.equal(isRelayConnectionDegraded("connecting"), false);
  assert.equal(isRelayConnectionDegraded("connected"), false);
});

test("isRelayConnectionDegraded — non-healthy states are degraded", () => {
  assert.equal(isRelayConnectionDegraded("reconnecting"), true);
  assert.equal(isRelayConnectionDegraded("stalled"), true);
  assert.equal(isRelayConnectionDegraded("disconnected"), true);
});
