import assert from "node:assert/strict";
import test from "node:test";

import { RelayConnectionStateEmitter } from "./relayConnectionStateEmitter.ts";

test("starts in the constructor-provided state", () => {
  const e = new RelayConnectionStateEmitter("connecting");
  assert.equal(e.get(), "connecting");
});

test("defaults to 'idle' when no initial state given", () => {
  const e = new RelayConnectionStateEmitter();
  assert.equal(e.get(), "idle");
});

test("set() notifies subscribers and updates state", () => {
  const e = new RelayConnectionStateEmitter("connecting");
  const seen = [];
  e.subscribe((s) => seen.push(s));
  // Initial replay: subscriber sees current state.
  assert.deepEqual(seen, ["connecting"]);

  e.set("connected");
  assert.deepEqual(seen, ["connecting", "connected"]);
  assert.equal(e.get(), "connected");
});

test("set() is a no-op when state is unchanged", () => {
  const e = new RelayConnectionStateEmitter("connected");
  const seen = [];
  e.subscribe((s) => seen.push(s));
  assert.deepEqual(seen, ["connected"]);

  e.set("connected");
  // No duplicate emission.
  assert.deepEqual(seen, ["connected"]);
});

test("unsubscribe stops further notifications", () => {
  const e = new RelayConnectionStateEmitter("idle");
  const seen = [];
  const unsub = e.subscribe((s) => seen.push(s));
  unsub();
  e.set("connecting");
  assert.deepEqual(seen, ["idle"]);
});

test("listener exceptions do not break other listeners", () => {
  const e = new RelayConnectionStateEmitter("idle");
  const seenA = [];
  const seenB = [];
  // Quiet the expected error log.
  const originalError = console.error;
  const errors = [];
  console.error = (...args) => errors.push(args);
  try {
    e.subscribe(() => {
      throw new Error("boom A");
    });
    e.subscribe((s) => seenA.push(s));
    e.subscribe((s) => seenB.push(s));
    e.set("connected");
  } finally {
    console.error = originalError;
  }
  assert.deepEqual(seenA, ["idle", "connected"]);
  assert.deepEqual(seenB, ["idle", "connected"]);
  // One during subscribe-replay, one during the set() emit.
  assert.ok(errors.length >= 2, "expected console.error to be called");
});

test("clear() drops listeners", () => {
  const e = new RelayConnectionStateEmitter("idle");
  const seen = [];
  e.subscribe((s) => seen.push(s));
  e.clear();
  e.set("connecting");
  assert.deepEqual(seen, ["idle"]);
});
