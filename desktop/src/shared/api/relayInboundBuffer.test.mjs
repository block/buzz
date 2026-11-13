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

test("EOSE cannot overtake events in a preceding connected IPC batch", async () => {
  const events = [];
  let completed;
  const inbound = createRelayInboundBuffer(async (batch) => {
    // RelayClientSession awaits handleWsMessage for every frame in a delivery.
    for (const frame of batch) {
      if (frame === "EOSE") completed = [...events];
      else events.push(frame);
      await Promise.resolve();
    }
  }, assert.fail);
  await inbound.drain();
  inbound.receive(Array.from({ length: 19 }, (_, i) => i + 1));
  inbound.receive([20, "EOSE"]);
  await inbound.drain();
  assert.deepEqual(
    completed,
    Array.from({ length: 20 }, (_, i) => i + 1),
  );
});

test("connected delivery stays ordered while an asynchronous frame is blocked", async () => {
  const gate = Promise.withResolvers();
  const handled = [];
  const inbound = createRelayInboundBuffer(async (message) => {
    handled.push(message);
    if (message === "first") await gate.promise;
  }, assert.fail);
  await inbound.drain();
  inbound.receive("first");
  await Promise.resolve();
  inbound.receive("second");
  await Promise.resolve();
  assert.deepEqual(handled, ["first"]);
  gate.resolve();
  await inbound.drain();
  assert.deepEqual(handled, ["first", "second"]);
  inbound.receive("after-drain");
  await inbound.drain();
  assert.deepEqual(handled, ["first", "second", "after-drain"]);
});

test("connected backlog is bounded and overflow discards queued frames", async () => {
  const gate = Promise.withResolvers();
  const handled = [];
  const errors = [];
  const inbound = createRelayInboundBuffer(
    async (message) => {
      handled.push(message);
      await gate.promise;
    },
    (error) => errors.push(error),
  );
  await inbound.drain();
  inbound.receive("blocked");
  await Promise.resolve();
  for (let i = 0; i <= MAX_PENDING_RELAY_FRAMES; i++) inbound.receive(i);
  await assert.rejects(inbound.overflow, /too many frames while processing/);
  gate.resolve();
  await inbound.drain();
  inbound.receive("after-failure");
  await inbound.drain();
  assert.deepEqual(handled, ["blocked"]);
  assert.equal(errors.length, 1);
});

test("a handler failure propagates and fences subsequent queued deliveries", async () => {
  const failure = new Error("broken frame");
  const errors = [];
  const handled = [];
  const inbound = createRelayInboundBuffer(
    async (message) => {
      handled.push(message);
      throw failure;
    },
    (error) => errors.push(error),
  );
  await inbound.drain();
  inbound.receive("bad");
  inbound.receive("queued");
  await assert.rejects(inbound.drain(), failure);
  inbound.receive("after-failure");
  await inbound.drain();
  assert.deepEqual(handled, ["bad"]);
  assert.deepEqual(errors, [failure]);
});
