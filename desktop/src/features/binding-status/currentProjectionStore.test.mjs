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
    connectionEpoch: "opaque-epoch-a",
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
    throwNextSchedules: (count = 1) => {
      schedulesToThrow = count;
    },
    delays,
  };
}

test("parses only the exact frozen narrow DTO", () => {
  const parsed = parseCurrentProjection(projection(), 100);

  assert.deepEqual(parsed, projection());
  assert.deepEqual(Object.keys(parsed), [
    "eventAuthorPubkey",
    "freshUntil",
    "connectionEpoch",
  ]);
  assert.equal(Object.isFrozen(parsed), true);
  assert.throws(() => {
    parsed.connectionEpoch = "mutated";
  }, TypeError);

  assert.equal(
    parseCurrentProjection(
      projection({ rawEvent: "must-not-cross", revision: 42 }),
      100,
    ),
    null,
  );
});

test("rejects noncanonical authors, invalid deadlines, and empty epochs", () => {
  const invalid = [
    null,
    [],
    projection({ eventAuthorPubkey: AUTHOR_A.toUpperCase() }),
    projection({ eventAuthorPubkey: "a".repeat(63) }),
    projection({ eventAuthorPubkey: `${"a".repeat(63)}g` }),
    projection({ freshUntil: 0 }),
    projection({ freshUntil: 100 }),
    projection({ freshUntil: 99 }),
    projection({ freshUntil: 100.5 }),
    projection({ freshUntil: Number.MAX_SAFE_INTEGER + 1 }),
    projection({ connectionEpoch: "" }),
  ];

  for (const candidate of invalid) {
    assert.equal(parseCurrentProjection(candidate, 100), null);
  }
  assert.equal(parseCurrentProjection(projection(), Number.NaN), null);
});

test("expires at the exclusive deadline without another input", () => {
  const timers = makeTimerHost(100.25);
  const store = createCurrentProjectionStore(timers.options);
  let changes = 0;
  store.subscribe(() => {
    changes += 1;
  });

  store.replaceFromNative(projection({ freshUntil: 101 }));
  assert.equal(store.getSnapshot()?.eventAuthorPubkey, AUTHOR_A);
  assert.deepEqual(timers.delays, [750]);

  timers.setNow(101);
  timers.fire(timers.pendingIds()[0]);
  assert.equal(store.getSnapshot(), null);
  assert.equal(changes, 2);
});

test("caps long timers and rearms after early fire or clock rollback", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore({
    ...timers.options,
    maxTimerDelayMs: 1_000,
  });

  store.replaceFromNative(projection({ freshUntil: 103 }));
  assert.deepEqual(timers.delays, [1_000]);

  timers.setNow(99);
  timers.fire(timers.pendingIds()[0]);
  assert.deepEqual(timers.delays, [1_000, 1_000]);
  assert.notEqual(store.getSnapshot(), null);

  timers.setNow(103);
  timers.fire(timers.pendingIds()[0]);
  assert.equal(store.getSnapshot(), null);
});

test("captured tokens reject old timers across replacement and clear", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);
  let changes = 0;
  store.subscribe(() => {
    changes += 1;
  });

  store.replaceFromNative(projection({ freshUntil: 110 }));
  const oldTimer = timers.pendingIds()[0];
  store.replaceFromNative(
    projection({
      eventAuthorPubkey: AUTHOR_B,
      freshUntil: 120,
      connectionEpoch: "opaque-epoch-b",
    }),
  );
  const currentTimer = timers.pendingIds()[0];

  timers.setNow(110);
  timers.fire(oldTimer);
  assert.equal(store.getSnapshot()?.eventAuthorPubkey, AUTHOR_B);
  assert.deepEqual(timers.pendingIds(), [currentTimer]);

  store.clear();
  store.clear();
  timers.setNow(120);
  timers.fire(currentTimer);
  assert.equal(store.getSnapshot(), null);
  assert.equal(changes, 3, "the second clear remains notification-idempotent");
});

test("invalid native input clears a current projection", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);

  store.replaceFromNative(projection());
  store.replaceFromNative({ ...projection(), connectionEpoch: "" });
  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(timers.pendingIds(), []);
});

test("a throwing subscriber cannot abort expiry or later subscribers", () => {
  const timers = makeTimerHost(100);
  const logCalls = [];
  const store = createCurrentProjectionStore({
    ...timers.options,
    onListenerError: (...args) => logCalls.push(args),
  });
  const observed = [];

  store.subscribe(() => {
    assert.equal(
      timers.pendingIds().length,
      store.getSnapshot() === null ? 0 : 1,
      "expiry is armed before a current snapshot is announced",
    );
    throw new Error("synthetic subscriber failure");
  });
  store.subscribe(() => observed.push(store.getSnapshot()));

  store.replaceFromNative(projection({ freshUntil: 101 }));
  assert.equal(store.getSnapshot()?.eventAuthorPubkey, AUTHOR_A);
  assert.equal(observed.length, 1);
  assert.deepEqual(logCalls, [[]], "logging receives no DTO or thrown value");

  timers.setNow(101);
  timers.fire(timers.pendingIds()[0]);
  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(observed, [projection({ freshUntil: 101 }), null]);
  assert.deepEqual(logCalls, [[], []]);
});

test("initial and replacement scheduler failures leave the store null", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore(timers.options);

  timers.throwNextSchedules();
  store.replaceFromNative(projection({ freshUntil: 110 }));
  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(timers.pendingIds(), []);

  store.replaceFromNative(projection({ freshUntil: 110 }));
  assert.notEqual(store.getSnapshot(), null);
  timers.throwNextSchedules();
  store.replaceFromNative(
    projection({
      eventAuthorPubkey: AUTHOR_B,
      freshUntil: 120,
      connectionEpoch: "opaque-epoch-b",
    }),
  );
  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(timers.pendingIds(), []);
});

test("rearm scheduler failure clears the current projection", () => {
  const timers = makeTimerHost(100);
  const store = createCurrentProjectionStore({
    ...timers.options,
    maxTimerDelayMs: 1_000,
  });

  store.replaceFromNative(projection({ freshUntil: 103 }));
  assert.notEqual(store.getSnapshot(), null);
  timers.throwNextSchedules();
  timers.setNow(100.5);
  timers.fire(timers.pendingIds()[0]);

  assert.equal(store.getSnapshot(), null);
  assert.deepEqual(timers.pendingIds(), []);
});
