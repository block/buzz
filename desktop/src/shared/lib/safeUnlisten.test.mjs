import assert from "node:assert/strict";
import test from "node:test";

import { safeUnlisten } from "./safeUnlisten.ts";

// Retries are driven through an injected host, so the tests never wait on real
// timers; handlers run immediately and we flush the microtask queue instead.
function immediateHost() {
  const delays = [];
  return {
    delays,
    host: {
      setTimeout: (handler, ms) => {
        delays.push(ms);
        handler();
      },
    },
  };
}

const flush = () => new Promise((resolve) => setImmediate(resolve));

test("calls the unlisten function once when it succeeds", async () => {
  const { host, delays } = immediateHost();
  let calls = 0;

  safeUnlisten(() => {
    calls += 1;
  }, host);
  await flush();

  assert.equal(calls, 1);
  assert.deepEqual(delays, [], "no retry was scheduled");
});

test("retries past the listener-registration race and stops on success", async () => {
  const { host, delays } = immediateHost();
  let calls = 0;

  safeUnlisten(() => {
    calls += 1;
    // Tauri's injected unregisterListener throws until the backend eval that
    // publishes this listener's id has landed in the webview.
    if (calls < 3) throw new TypeError("listeners[eventId] is undefined");
  }, host);
  await flush();

  assert.equal(calls, 3, "keeps trying until the listener is actually removed");
  assert.deepEqual(delays, [0, 16], "backs off between attempts");
});

test("retries a rejected async unlisten", async () => {
  const { host } = immediateHost();
  let calls = 0;

  safeUnlisten(async () => {
    calls += 1;
    if (calls < 2) throw new Error("not registered yet");
  }, host);
  await flush();

  assert.equal(calls, 2);
});

test("gives up after exhausting retries without rejecting", async () => {
  const { host, delays } = immediateHost();
  const originalDebug = console.debug;
  const debugCalls = [];
  console.debug = (...args) => debugCalls.push(args);
  let calls = 0;

  try {
    safeUnlisten(() => {
      calls += 1;
      throw new Error("always fails");
    }, host);
    await flush();
  } finally {
    console.debug = originalDebug;
  }

  assert.equal(calls, 5, "one initial attempt plus every configured retry");
  assert.deepEqual(delays, [0, 16, 64, 256]);
  assert.equal(debugCalls.length, 1, "logs once instead of throwing");
});

test("ignores a missing unlisten function", async () => {
  const { host } = immediateHost();

  safeUnlisten(null, host);
  safeUnlisten(undefined, host);
  await flush();
});
