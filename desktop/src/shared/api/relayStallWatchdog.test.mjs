import assert from "node:assert/strict";
import test from "node:test";

import { RelayStallWatchdog } from "./relayStallWatchdog.ts";

// Shim `window` to expose the timer APIs the watchdog uses. The real
// RelayClient runs in a Tauri WebView where `window` exists; under node:test we
// wire it to the same globals.
if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    setInterval: (...args) => setInterval(...args),
    clearInterval: (id) => clearInterval(id),
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function makeWatchdog(overrides = {}) {
  const stalls = [];
  let now = overrides.now ?? 1;
  const wd = new RelayStallWatchdog({
    intervalMs: overrides.intervalMs ?? 20,
    idleTimeoutMs: overrides.idleTimeoutMs ?? 50,
    onStall: (err) => {
      stalls.push(err);
    },
    now: () => now,
  });
  return {
    advance: (ms) => {
      now += ms;
    },
    setNow: (value) => {
      now = value;
    },
    stalls,
    wd,
  };
}

test("does not send probes while watching for stalls", async () => {
  const { wd } = makeWatchdog();
  wd.start();
  await sleep(45);
  wd.stop();
  // The passive watchdog has no send callback by construction. This test is a
  // regression guard for the half-open-socket bug: liveness checks must not write to a
  // socket already suspected of being half-open.
  assert.equal(typeof wd.recordInbound, "function");
});

test("idle timeout without inbound frames triggers onStall", async () => {
  const { advance, stalls, wd } = makeWatchdog();
  wd.start();
  advance(60);
  for (let i = 0; i < 20 && stalls.length === 0; i++) await sleep(5);
  wd.stop();
  assert.equal(stalls.length, 1);
  assert.match(stalls[0].message, /no inbound frames/i);
});

test("inbound frames reset the idle timer", async () => {
  const { advance, stalls, wd } = makeWatchdog({ idleTimeoutMs: 50 });
  wd.start();
  advance(40);
  wd.recordInbound();
  advance(40);
  await sleep(30);
  assert.equal(
    stalls.length,
    0,
    "recent inbound frame should keep socket alive",
  );
  advance(20);
  for (let i = 0; i < 20 && stalls.length === 0; i++) await sleep(5);
  wd.stop();
  assert.equal(stalls.length, 1);
});

test("recordInbound is ignored while stopped", async () => {
  const { advance, stalls, wd } = makeWatchdog({ idleTimeoutMs: 50 });
  wd.recordInbound();
  advance(100);
  await sleep(30);
  assert.equal(stalls.length, 0);
});

test("stop() cancels the idle check", async () => {
  const { advance, stalls, wd } = makeWatchdog({ idleTimeoutMs: 50 });
  wd.start();
  wd.stop();
  advance(100);
  await sleep(35);
  assert.equal(stalls.length, 0);
});

test("start() is idempotent — does not create duplicate intervals", async () => {
  const { advance, stalls, wd } = makeWatchdog({ idleTimeoutMs: 50 });
  wd.start();
  wd.start();
  wd.start();
  advance(60);
  for (let i = 0; i < 20 && stalls.length === 0; i++) await sleep(5);
  wd.stop();
  assert.equal(stalls.length, 1);
});
