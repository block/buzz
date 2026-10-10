import assert from "node:assert/strict";
import test from "node:test";

import { connectWebSocketWithTimeout } from "./relayConnectTimeout.ts";

// Shim the WebView `window` timer APIs, same pattern as the stall-watchdog test.
if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    setTimeout: (...args) => setTimeout(...args),
    clearTimeout: (id) => clearTimeout(id),
  };
}

// A `plugin:websocket|connect` invoke that never settles — the stuck state from
// issue #3975, where a dead connection holds the plugin's global
// connection-manager mutex and any later `connect()` registration waits forever.
const neverResolvingInvoke = () => new Promise(() => {});

const immediateInvoke = (command) => {
  if (command === "plugin:websocket|connect") return Promise.resolve(42);
  return Promise.reject(new Error(`unexpected invoke: ${command}`));
};

test("rejects with a timeout error when the plugin connect invoke never settles (#3975)", async () => {
  const start = Date.now();
  let rejected;
  try {
    await connectWebSocketWithTimeout(
      neverResolvingInvoke,
      "ws://127.0.0.1:4869",
      null,
      150,
    );
  } catch (error) {
    rejected = error;
  }
  const elapsed = Date.now() - start;

  assert.ok(rejected, "expected the timeout to reject");
  assert.match(
    rejected.message,
    /connect invoke timed out|timed out/i,
    `expected a timeout error, got: ${rejected.message}`,
  );
  assert.match(rejected.message, /150ms/, "error should echo the timeout budget");
  assert.ok(
    elapsed >= 130 && elapsed < 600,
    `should settle near the 150ms timeout (was ${elapsed}ms)`,
  );
});

test("resolves with the plugin wsId when the connect invoke responds", async () => {
  const wsId = await connectWebSocketWithTimeout(
    immediateInvoke,
    "ws://127.0.0.1:4869",
    null,
    150,
  );
  assert.equal(wsId, 42);
});

test("clears its timer so a fast resolve does not later fire the timeout", async () => {
  let cleared = false;
  const clearTimeoutFn = (id) => {
    cleared = true;
    clearTimeout(id);
  };
  const wsId = await connectWebSocketWithTimeout(
    immediateInvoke,
    "ws://127.0.0.1:4869",
    null,
    5_000, // a long budget that must never fire because the invoke wins the race
    undefined, // default setTimeout
    clearTimeoutFn,
  );
  assert.equal(wsId, 42);
  assert.ok(cleared, "timeout must be cleared when the invoke wins the race");

  // Let a beat pass beyond the (cleared) timeout; nothing should throw.
  await new Promise((r) => setTimeout(r, 10));
});
