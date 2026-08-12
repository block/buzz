import assert from "node:assert/strict";
import test from "node:test";

import {
  createCurrentProjectionStore,
  parseCurrentProjection,
} from "./currentProjectionStore.ts";

const AUTHOR_A = "ab".repeat(32);
const AUTHOR_B = "cd".repeat(32);

function projection(overrides = {}) {
  return {
    eventAuthorPubkey: AUTHOR_A,
    freshUntil: 200,
    ...overrides,
  };
}

function makeTimerHost(initialNow = 100) {
  let now = initialNow;
  let nextId = 1;
  const pending = new Map();
  const callbacks = new Map();
  const delays = [];
  let schedulesToThrow = 0;

  return {
    options: {
      nowSeconds: () => now,
      setTimeout: (callback, delayMs) => {
        if (schedulesToThrow > 0) {
          schedulesToThrow -= 1;
          throw new Error("synthetic scheduler failure");
        }
        const id = nextId++;
        pending.set(id, callback);
        callbacks.set(id, callback);
        delays.push(delayMs);
        return id;
      },
      clearTimeout: (id) => pending.delete(id),
    },
    setNow: (value) => {
      now = value;
    },
    fire: (id) => {
      pending.delete(id);
      callbacks.get(id)?.();
    },
    pendingIds: () => [...pending.keys()],
    throwNextSchedule: () => {
      schedulesToThrow += 1;
    },
    delays,
  };
}

test("parses only the exact frozen two-key DTO", () => {
  const parsed = parseCurrentProjection(projection(), 100);
  assert.deepEqual(parsed, projection());
  assert.equal(Object.isFrozen(parsed), true);
  for (const extra of [
    { connectionEpoch: "11111111-1111-4111-8111-111111111111" },
    { rawEvent: "must-not-cross" },
    { revision: 42 },
  ]) {
    assert.equal(parseCurrentProjection(projection(extra), 100), null);
  }
});

test("rejects malformed authors and deadlines", () => {
  for (const candidate of [
    null,
    [],
    projection({ eventAuthorPubkey: AUTHOR_A.toUpperCase() }),
    projection({ eventAuthorPubkey: "a".repeat(63) }),
    projection({ freshUntil: 100 }),
    projection({ freshUntil: 100.5 }),
    projection({ freshUntil: 401 }),
  ]) {
    assert.equal(parseCurrentProjection(candidate, 100), null);
  }
});

test("render-time reads hide exact-bound expiry before a throttled callback", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);
  let changes = 0;
  store.subscribe(() => {
    changes += 1;
  });

  store.replaceFromNative(projection({ freshUntil: 101 }));
  const delayedTimer = timers.pendingIds()[0];
  assert.notEqual(store.getSnapshot(), null);

  timers.setNow(101);
  assert.equal(store.getSnapshot(), null);
  assert.equal(changes, 1);

  timers.fire(delayedTimer);
  assert.equal(store.getSnapshot(), null);
  assert.equal(changes, 2);
});

test("captured tokens reject stale expiry after replacement and clear", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);

  store.replaceFromNative(projection({ freshUntil: 110 }));
  const oldTimer = timers.pendingIds()[0];
  store.replaceFromNative(
    projection({ eventAuthorPubkey: AUTHOR_B, freshUntil: 120 }),
  );
  const currentTimer = timers.pendingIds()[0];

  timers.setNow(110);
  timers.fire(oldTimer);
  assert.equal(store.getSnapshot()?.eventAuthorPubkey, AUTHOR_B);
  assert.deepEqual(timers.pendingIds(), [currentTimer]);

  store.clear();
  timers.setNow(120);
  timers.fire(currentTimer);
  assert.equal(store.getSnapshot(), null);
});

test("invalid native replacement and scheduler failure clear current state", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);

  store.replaceFromNative(projection());
  store.replaceFromNative({ ...projection(), connectionEpoch: "forbidden" });
  assert.equal(store.getSnapshot(), null);

  timers.throwNextSchedule();
  store.replaceFromNative(projection({ freshUntil: 110 }));
  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(timers.pendingIds(), []);
});
