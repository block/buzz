import assert from "node:assert/strict";
import test from "node:test";

import { handleRelayClosed } from "./relayClosedRecovery.ts";

test("production CLOSED handler rejects history once and clears its timeout", () => {
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
