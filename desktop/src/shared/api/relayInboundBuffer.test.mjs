import assert from "node:assert/strict";
import test from "node:test";

import {
  createRelayInboundBuffer,
  MAX_PENDING_RELAY_FRAMES,
} from "./relayInboundBuffer.ts";

test("drains queued frames in order, including frames received during drain", async () => {
  const handled = [];
  let releaseFirst;
  const firstBlocked = new Promise((resolve) => {
    releaseFirst = resolve;
  });
  const inbound = createRelayInboundBuffer(async (message) => {
    handled.push(message);
    if (message === "first") await firstBlocked;
  }, assert.fail);

  inbound.receive("first");
  const drain = inbound.drain();
  inbound.receive("second");
  releaseFirst();
  await drain;
  inbound.receive("live");
  await new Promise((resolve) => setTimeout(resolve));

  assert.deepEqual(handled, ["first", "second", "live"]);
});

test("rejects, resets, and stops accepting frames when the cap is exceeded", async () => {
  let overflowError;
  const inbound = createRelayInboundBuffer(
    async () => {},
    (error) => {
      overflowError = error;
    },
  );
  for (let i = 0; i < MAX_PENDING_RELAY_FRAMES + 1; i++) inbound.receive(i);

  await assert.rejects(
    inbound.overflow,
    /Relay sent too many frames while connecting/,
  );
  assert.match(overflowError.message, /too many frames/);
});
