import assert from "node:assert/strict";
import test from "node:test";

import { RelayStallWatchdog } from "./relayStallWatchdog.ts";

// Shim `window` to expose the timer + crypto APIs the watchdog uses. The
// real RelayClient runs in a Tauri WebView where `window` exists; under
// node:test we wire it to the same globals.
if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    setInterval: (...args) => setInterval(...args),
    clearInterval: (id) => clearInterval(id),
    setTimeout: (...args) => setTimeout(...args),
    clearTimeout: (id) => clearTimeout(id),
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function makeWatchdog(overrides = {}) {
  const sends = [];
  const stalls = [];
  const wd = new RelayStallWatchdog({
    intervalMs: overrides.intervalMs ?? 30,
    probeTimeoutMs: overrides.probeTimeoutMs ?? 30,
    sendRaw:
      overrides.sendRaw ??
      (async (payload) => {
        sends.push(payload);
      }),
    onStall: (err) => {
      stalls.push(err);
    },
    now: overrides.now,
  });
  return { wd, sends, stalls };
}

test("first probe carries the expected NIP-01 REQ shape", async () => {
  const { wd, sends } = makeWatchdog();
  wd.start();
  // Wait until a probe is observed.
  for (let i = 0; i < 50 && sends.length === 0; i++) await sleep(5);
  wd.stop();
  assert.equal(sends.length, 1);
  const [verb, subId, filter] = sends[0];
  assert.equal(verb, "REQ");
  assert.match(subId, /^probe-/);
  assert.deepEqual(filter.kinds, [9999]);
  assert.equal(filter.limit, 0);
  assert.ok(typeof filter.since === "number");
});

test("EOSE for the current probe clears in-flight + lets the next probe fire", async () => {
  const { wd, sends, stalls } = makeWatchdog();
  wd.start();
  for (let i = 0; i < 50 && sends.length === 0; i++) await sleep(5);
  const firstSubId = sends[0][1];
  // Resolve the probe.
  assert.equal(wd.handleEose(firstSubId), true);
  // Within the next interval+probe window, another probe should fire.
  for (let i = 0; i < 50 && sends.length < 2; i++) await sleep(5);
  wd.stop();
  assert.ok(sends.length >= 2, `expected ≥2 probes, got ${sends.length}`);
  assert.equal(stalls.length, 0, "no stall expected when EOSE arrives");
});

test("EOSE for a non-probe subId returns false", () => {
  const { wd } = makeWatchdog();
  assert.equal(wd.handleEose("live-abc"), false);
});

test("timeout without EOSE triggers onStall", async () => {
  const { wd, stalls } = makeWatchdog();
  wd.start();
  // intervalMs (30) before first send + probeTimeoutMs (30) — wait a bit
  // past their sum.
  for (let i = 0; i < 50 && stalls.length === 0; i++) await sleep(10);
  wd.stop();
  assert.ok(stalls.length >= 1, "expected at least one stall");
  assert.match(stalls[0].message, /stalled/i);
});

test("send-side failure triggers onStall immediately", async () => {
  const { wd, stalls } = makeWatchdog({
    sendRaw: async () => {
      throw new Error("ws is dead");
    },
  });
  wd.start();
  for (let i = 0; i < 50 && stalls.length === 0; i++) await sleep(5);
  wd.stop();
  assert.ok(stalls.length >= 1, "expected stall on send failure");
  assert.match(stalls[0].message, /ws is dead/);
});

test("stop() cancels a pending stall timeout", async () => {
  const { wd, sends, stalls } = makeWatchdog();
  wd.start();
  for (let i = 0; i < 50 && sends.length === 0; i++) await sleep(5);
  // Probe is in-flight; stop before it can time out.
  wd.stop();
  // Wait well past the timeout window.
  await sleep(80);
  assert.equal(stalls.length, 0, "stop() should cancel the pending stall");
});

test("start() is idempotent — does not create duplicate intervals", async () => {
  const { wd, sends } = makeWatchdog({ intervalMs: 25, probeTimeoutMs: 200 });
  wd.start();
  wd.start();
  wd.start();
  // Allow one probe to fire and resolve it so the *next* probe can fire if
  // the interval was somehow doubled.
  for (let i = 0; i < 50 && sends.length === 0; i++) await sleep(5);
  wd.handleEose(sends[0][1]);
  // Within one more interval window, exactly one more probe should fire
  // (not two), which is the contract for `start()` being idempotent.
  await sleep(45);
  wd.stop();
  assert.ok(
    sends.length <= 2,
    `expected ≤2 probes despite triple-start(), got ${sends.length}`,
  );
});
