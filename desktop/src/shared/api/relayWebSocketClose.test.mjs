import assert from "node:assert/strict";
import test from "node:test";

import { closeAllWebSockets, closeWebSocket } from "./relayWebSocketClose.ts";

test("closeWebSocket invokes authoritative native disconnect", async () => {
  const calls = [];
  await closeWebSocket(42, "community switch", async (cmd, args) => {
    calls.push({ cmd, args });
  });

  assert.deepEqual(calls, [
    { cmd: "plugin:websocket|disconnect", args: { id: 42 } },
  ]);
});

test("closeWebSocket is idempotent when the native socket is gone", async () => {
  await closeWebSocket(7, "connection reset", async () => {
    throw new Error("WebSocket connection not found");
  });
});

test("closeAllWebSockets invokes native process-wide teardown", async () => {
  const calls = [];
  await closeAllWebSockets(async (cmd, args) => calls.push({ cmd, args }));
  assert.deepEqual(calls, [
    { cmd: "plugin:websocket|disconnect_all", args: undefined },
  ]);
});
