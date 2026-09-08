import assert from "node:assert/strict";
import test from "node:test";

import {
  BOOT_SWEEP_FLOOR_MS,
  LOCAL_STORAGE_SWEEP_RULES,
  SWEEP_CHUNK_ENTRY_BUDGET,
  SWEEP_FALLBACK_TIMER_MS,
  SWEEP_IDLE_TIMEOUT_MS,
  SWEEP_LARGE_VALUE_DEFER_BYTES,
  startLocalStorageSweep,
  sweepStaleLocalStorage,
} from "./localStorageSweep.ts";

const DAY_MS = 24 * 60 * 60 * 1_000;

function makeLocalStorage(entries = []) {
  const store = new Map(entries);
  return {
    store,
    get length() {
      return store.size;
    },
    key: (index) => [...store.keys()][index] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
  };
}

function installWindow(localStorage, overrides = {}) {
  globalThis.window = { localStorage, ...overrides };
}

const snapshot = (updatedAt) => JSON.stringify({ updatedAt, payload: "cache" });

test("sweeps stale whitelisted caches and keeps fresh or durable state", () => {
  const now = 100 * DAY_MS;
  const entries = LOCAL_STORAGE_SWEEP_RULES.flatMap(
    ({ keyPrefix, maxAgeMs }, index) => [
      [`${keyPrefix}stale-a-${index}`, snapshot(now - maxAgeMs)],
      [`${keyPrefix}stale-b-${index}`, snapshot(now - maxAgeMs - DAY_MS)],
      [`${keyPrefix}fresh-${index}`, snapshot(now - maxAgeMs + 1)],
    ],
  );
  entries.push(["buzz-communities", snapshot(0)]);
  entries.push(["buzz-theme", snapshot(0)]);
  entries.push(["buzz-self-profile.v1:offline", snapshot(0)]);
  const localStorage = makeLocalStorage(entries);
  installWindow(localStorage);

  assert.equal(
    sweepStaleLocalStorage(now),
    LOCAL_STORAGE_SWEEP_RULES.length * 2,
  );
  for (const { keyPrefix } of LOCAL_STORAGE_SWEEP_RULES) {
    assert.equal(
      [...localStorage.store.keys()].some((key) =>
        key.startsWith(`${keyPrefix}stale-`),
      ),
      false,
    );
    assert.equal(
      [...localStorage.store.keys()].some((key) =>
        key.startsWith(`${keyPrefix}fresh-`),
      ),
      true,
    );
  }
  assert.equal(localStorage.getItem("buzz-communities"), snapshot(0));
  assert.equal(localStorage.getItem("buzz-theme"), snapshot(0));
  assert.equal(
    localStorage.getItem("buzz-self-profile.v1:offline"),
    snapshot(0),
  );
});

test("uses the newest per-profile timestamp for user-label cache buckets", () => {
  const now = 100 * DAY_MS;
  const localStorage = makeLocalStorage([
    [
      "buzz-user-labels.v1:all-stale",
      JSON.stringify({
        profiles: {
          first: { updatedAt: now - 20 * DAY_MS },
          second: { updatedAt: now - 14 * DAY_MS },
        },
      }),
    ],
    [
      "buzz-user-labels.v1:one-fresh",
      JSON.stringify({
        profiles: {
          stale: { updatedAt: now - 20 * DAY_MS },
          fresh: { updatedAt: now - DAY_MS },
        },
      }),
    ],
  ]);
  installWindow(localStorage);

  assert.equal(sweepStaleLocalStorage(now), 1);
  assert.equal(localStorage.getItem("buzz-user-labels.v1:all-stale"), null);
  assert.notEqual(localStorage.getItem("buzz-user-labels.v1:one-fresh"), null);
});

test("leaves malformed and timestamp-free cache entries untouched", () => {
  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:malformed", "not json"],
    ["buzz-channels.v1:no-timestamp", JSON.stringify({ payload: "cache" })],
    ["buzz-observed-unread.v1:bad-timestamp", snapshot(Number.NaN)],
  ]);
  installWindow(localStorage);

  assert.equal(sweepStaleLocalStorage(100 * DAY_MS), 0);
  assert.equal(localStorage.store.size, 3);
});

test("storage access failures warn and never escape", () => {
  const originalWarn = console.warn;
  const warnings = [];
  console.warn = (...args) => warnings.push(args);
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {},
  });
  Object.defineProperty(globalThis.window, "localStorage", {
    configurable: true,
    get() {
      throw new Error("SecurityError");
    },
  });

  try {
    assert.doesNotThrow(() => sweepStaleLocalStorage(100 * DAY_MS));
    assert.equal(sweepStaleLocalStorage(100 * DAY_MS), 0);
    assert.equal(warnings.length, 2);
  } finally {
    console.warn = originalWarn;
  }
});

test("scheduler setup failures warn and never escape startup", () => {
  const originalWarn = console.warn;
  const warnings = [];
  console.warn = (...args) => warnings.push(args);
  installWindow(makeLocalStorage(), {
    clearInterval() {},
    setInterval() {
      throw new Error("timer unavailable");
    },
  });

  try {
    let stop;
    assert.doesNotThrow(() => {
      stop = startLocalStorageSweep();
    });
    assert.doesNotThrow(() => stop());
    assert.equal(warnings.length, 1);
  } finally {
    console.warn = originalWarn;
  }
});

test("fallback path: boot-floor setTimeout fires first, then per-slice setTimeout processes entries", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  const timeouts = new Map();
  let nextId = 50;
  globalThis.setTimeout = (callback, delay) => {
    const id = nextId++;
    timeouts.set(id, { callback, delay });
    return id;
  };
  globalThis.clearTimeout = (id) => timeouts.delete(id);

  // Stale entry with updatedAt=0 (older than any 14-day TTL)
  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:startup", snapshot(0)],
  ]);
  installWindow(localStorage, {
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    // One boot-floor timer scheduled; no sweep yet.
    assert.equal(timeouts.size, 1);
    const [bootId, bootEntry] = [...timeouts.entries()][0];
    assert.equal(bootEntry.delay, BOOT_SWEEP_FLOOR_MS);
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
      "sweep must not run before boot floor",
    );

    // Fire boot timer — sweepChunked snapshots keys and schedules a slice.
    timeouts.delete(bootId);
    bootEntry.callback();

    // One per-slice fallback timer now pending; still no removal.
    assert.equal(timeouts.size, 1);
    const [, sliceEntry] = [...timeouts.entries()][0];
    assert.equal(sliceEntry.delay, SWEEP_FALLBACK_TIMER_MS);
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
      "sweep must not run before slice fires",
    );

    // Fire slice — stale entry removed.
    sliceEntry.callback();
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
      "stale entry removed after slice fires",
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("scheduler: boot-floor gate, idle callback with SWEEP_IDLE_TIMEOUT_MS, hourly interval, no visibilitychange listener", () => {
  const originalNow = Date.now;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const now = 100 * DAY_MS;
  Date.now = () => now;

  const intervals = new Map();
  const idleCallbacks = new Map();
  let nextIntervalId = 1;
  let nextIdleId = 100;

  // Capture the boot setTimeout.
  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 9999;
  };
  globalThis.clearTimeout = () => {};

  // Track visibilitychange listeners to assert none are registered.
  const visibilityListeners = [];

  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:stale", snapshot(now - 14 * DAY_MS)],
  ]);
  installWindow(localStorage, {
    requestIdleCallback(callback, options) {
      const id = nextIdleId++;
      idleCallbacks.set(id, { callback, options });
      return id;
    },
    cancelIdleCallback(id) {
      idleCallbacks.delete(id);
    },
    setInterval(callback, delay) {
      const id = nextIntervalId++;
      intervals.set(id, { callback, delay });
      return id;
    },
    clearInterval: (id) => intervals.delete(id),
  });
  globalThis.document = {
    addEventListener(event, fn) {
      if (event === "visibilitychange") visibilityListeners.push(fn);
    },
    removeEventListener() {},
    visibilityState: "visible",
  };

  const stop = startLocalStorageSweep();
  try {
    // Boot floor: no idle callbacks before the boot timer fires.
    assert.equal(idleCallbacks.size, 0, "no idle callback before boot floor");
    assert.equal(visibilityListeners.length, 0, "no visibilitychange listener");
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:stale"),
      null,
      "sweep not run before boot floor",
    );

    // Fire boot timer — sweepChunked schedules first idle callback.
    assert.ok(bootCallback, "boot timer was registered");
    bootCallback();
    assert.equal(idleCallbacks.size, 1, "idle callback scheduled after boot");
    const [firstId, firstEntry] = [...idleCallbacks.entries()][0];
    assert.equal(
      firstEntry.options.timeout,
      SWEEP_IDLE_TIMEOUT_MS,
      "idle callback uses SWEEP_IDLE_TIMEOUT_MS, not a forcing 1500ms",
    );

    // Fire idle callback with ample time remaining — sweep completes in one slice.
    firstEntry.callback({ timeRemaining: () => 50 });
    idleCallbacks.delete(firstId);
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:stale"),
      null,
      "stale entry removed after idle callback fires",
    );

    // Hourly interval is registered.
    assert.equal(intervals.size, 1);
    const [{ callback: intervalCallback, delay }] = intervals.values();
    assert.equal(delay, 60 * 60 * 1_000);

    // Hourly fires — schedules another idle callback sweep.
    localStorage.setItem(
      "buzz-channel-messages.v1:interval",
      snapshot(now - 14 * DAY_MS),
    );
    intervalCallback();
    assert.equal(
      idleCallbacks.size,
      1,
      "idle callback scheduled for hourly sweep",
    );
    const hourlyEntry = [...idleCallbacks.values()][0];
    hourlyEntry.callback({ timeRemaining: () => 50 });
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:interval"),
      null,
      "hourly sweep removes stale entry",
    );

    // Visibility change does NOT trigger sweep — listener was never registered.
    localStorage.setItem(
      "buzz-channel-messages.v1:vis",
      snapshot(now - 14 * DAY_MS),
    );
    assert.equal(
      visibilityListeners.length,
      0,
      "visibilitychange listener absent",
    );
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:vis"),
      null,
      "visibility change does not trigger sweep",
    );
  } finally {
    stop();
    Date.now = originalNow;
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
    globalThis.document = undefined;
  }
});

test("chunked sweep: keys processed across multiple idle slices; stale entries removed only at end", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 1;
  };
  globalThis.clearTimeout = () => {};

  const idleCallbacks = [];
  const now = 100 * DAY_MS;

  // Create more entries than SWEEP_CHUNK_ENTRY_BUDGET so multiple slices fire.
  const entryCount = SWEEP_CHUNK_ENTRY_BUDGET + 5;
  const entries = Array.from({ length: entryCount }, (_, i) => [
    `buzz-channel-messages.v1:key-${i}`,
    snapshot(now - 14 * DAY_MS), // all stale
  ]);
  const localStorage = makeLocalStorage(entries);

  installWindow(localStorage, {
    requestIdleCallback(callback, _options) {
      idleCallbacks.push(callback);
      return idleCallbacks.length;
    },
    cancelIdleCallback() {},
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    // Fire boot timer.
    assert.ok(bootCallback);
    bootCallback();

    // First idle callback scheduled — fire it with zero time remaining so it
    // processes exactly one entry (min-progress guarantee) and reschedules.
    assert.equal(idleCallbacks.length, 1, "first idle callback scheduled");
    idleCallbacks[0]({ timeRemaining: () => 0 });

    // With zero time remaining the slice stops after one entry;
    // stale keys not yet removed (second slice pending).
    const removedAfterFirst = entries.filter(
      ([key]) => localStorage.getItem(key) === null,
    ).length;
    assert.equal(
      removedAfterFirst,
      0,
      "no keys removed until all slices complete",
    );

    // A second idle callback was scheduled for the remaining entries.
    assert.equal(idleCallbacks.length, 2, "second idle callback scheduled");
    idleCallbacks[1]({ timeRemaining: () => 50 }); // ample time — completes all

    // All stale keys now removed.
    const remaining = entries.filter(
      ([key]) => localStorage.getItem(key) !== null,
    );
    assert.equal(
      remaining.length,
      0,
      "all stale entries removed after final slice",
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("chunked sweep: keys deleted between snapshot and slice are silently skipped", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 1;
  };
  globalThis.clearTimeout = () => {};

  const idleCallbacks = [];
  // Use real Date.now() so timestamps are relative to the actual clock.
  const now = Date.now();

  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:gone", snapshot(now - 14 * DAY_MS)],
    ["buzz-channel-messages.v1:fresh", snapshot(now - DAY_MS)],
  ]);

  installWindow(localStorage, {
    requestIdleCallback(callback, _options) {
      idleCallbacks.push(callback);
      return idleCallbacks.length;
    },
    cancelIdleCallback() {},
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    bootCallback();
    assert.equal(idleCallbacks.length, 1);

    // Delete "gone" externally before the slice fires.
    localStorage.store.delete("buzz-channel-messages.v1:gone");

    // Slice processes — should not throw on missing key.
    assert.doesNotThrow(() => {
      idleCallbacks[0]({ timeRemaining: () => 50 });
    });

    // "fresh" key untouched.
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:fresh"),
      null,
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

// --- New hardening tests ---

test("chunked sweep: key rewritten fresh before batch removal is not removed", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 1;
  };
  globalThis.clearTimeout = () => {};

  const idleCallbacks = [];
  const now = Date.now();

  const staleValue = JSON.stringify({
    updatedAt: now - 14 * DAY_MS,
    payload: "old",
  });
  const freshValue = JSON.stringify({
    updatedAt: now - DAY_MS,
    payload: "new",
  });

  // "target" will be rewritten fresh between slices; "other" stays stale.
  // Two entries force two slices when timeRemaining() === 0 (1 entry per slice).
  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:target", staleValue],
    ["buzz-channel-messages.v1:other", staleValue],
  ]);

  installWindow(localStorage, {
    requestIdleCallback(callback, _options) {
      idleCallbacks.push(callback);
      return idleCallbacks.length;
    },
    cancelIdleCallback() {},
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    bootCallback();
    assert.equal(idleCallbacks.length, 1);

    // Slice 1: zero budget → processes exactly one entry ("target", judged stale).
    idleCallbacks[0]({ timeRemaining: () => 0 });

    // App rewrites "target" to a fresh value before slice 2 fires.
    localStorage.setItem("buzz-channel-messages.v1:target", freshValue);

    assert.equal(idleCallbacks.length, 2, "second slice scheduled");

    // Slice 2: processes "other", then batch-removes. Re-check on "target"
    // finds it fresh — must not be removed.
    idleCallbacks[1]({ timeRemaining: () => 50 });

    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:target"),
      freshValue,
      "key rewritten fresh before batch removal must not be removed",
    );
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:other"),
      null,
      "still-stale key must be removed",
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("chunked sweep: getItem error on one key does not abort sweep; stale siblings still removed", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const originalWarn = console.warn;
  const warnings = [];
  console.warn = (...args) => warnings.push(args);

  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 1;
  };
  globalThis.clearTimeout = () => {};

  const idleCallbacks = [];
  const now = Date.now();

  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:error-key", "value"],
    ["buzz-channel-messages.v1:stale-key", snapshot(now - 14 * DAY_MS)],
    ["buzz-channel-messages.v1:fresh-key", snapshot(now - DAY_MS)],
  ]);

  // Override getItem so that reading "error-key" throws.
  const realGetItem = localStorage.getItem.bind(localStorage);
  localStorage.getItem = (key) => {
    if (key === "buzz-channel-messages.v1:error-key") {
      throw new Error("storage error");
    }
    return realGetItem(key);
  };

  installWindow(localStorage, {
    requestIdleCallback(callback, _options) {
      idleCallbacks.push(callback);
      return idleCallbacks.length;
    },
    cancelIdleCallback() {},
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    bootCallback();
    assert.equal(idleCallbacks.length, 1);

    // Fire with ample time — all entries processed in one slice.
    idleCallbacks[0]({ timeRemaining: () => 50 });

    // Stale key removed despite the per-key error on its sibling.
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:stale-key"),
      null,
      "stale-key must be removed despite error on error-key",
    );
    // Fresh key untouched.
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:fresh-key"),
      null,
      "fresh-key must not be removed",
    );
    // Exactly one warning emitted for the failing key.
    const sweepWarnings = warnings.filter((w) =>
      String(w[0]).includes("[localStorageSweep]"),
    );
    assert.equal(
      sweepWarnings.length,
      1,
      "exactly one warning per sweep for key errors",
    );
  } finally {
    stop();
    console.warn = originalWarn;
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("fallback path: multiple slices when entries exceed SWEEP_CHUNK_ENTRY_BUDGET; sweep converges", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  const timeouts = [];
  let nextId = 200;
  globalThis.setTimeout = (callback, delay) => {
    const id = nextId++;
    timeouts.push({ id, callback, delay });
    return id;
  };
  globalThis.clearTimeout = (id) => {
    const idx = timeouts.findIndex((t) => t.id === id);
    if (idx !== -1) timeouts.splice(idx, 1);
  };

  const now = Date.now();
  const entryCount = SWEEP_CHUNK_ENTRY_BUDGET + 5;
  const entries = Array.from({ length: entryCount }, (_, i) => [
    `buzz-channel-messages.v1:key-${i}`,
    snapshot(now - 14 * DAY_MS), // all stale
  ]);
  const localStorage = makeLocalStorage(entries);

  // No requestIdleCallback on window → forces fallback path.
  installWindow(localStorage, {
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    // Boot timer registered.
    const bootTimer = timeouts.find((t) => t.delay === BOOT_SWEEP_FLOOR_MS);
    assert.ok(bootTimer, "boot timer registered");
    timeouts.splice(timeouts.indexOf(bootTimer), 1);
    bootTimer.callback();

    // Process all per-slice timers until no more are scheduled.
    let slicesProcessed = 0;
    const MAX_SLICES = 20;
    while (slicesProcessed < MAX_SLICES) {
      const sliceTimer = timeouts.find(
        (t) => t.delay === SWEEP_FALLBACK_TIMER_MS,
      );
      if (!sliceTimer) break;
      timeouts.splice(timeouts.indexOf(sliceTimer), 1);
      sliceTimer.callback();
      slicesProcessed++;
    }

    // entryCount > SWEEP_CHUNK_ENTRY_BUDGET → at least 2 slices required.
    assert.ok(
      slicesProcessed >= 2,
      `expected ≥2 slices, got ${slicesProcessed}`,
    );

    // All stale entries removed.
    const remaining = entries.filter(
      ([key]) => localStorage.getItem(key) !== null,
    );
    assert.equal(
      remaining.length,
      0,
      "all stale entries must be removed via fallback path",
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("chunked sweep: large values deferred once on zero-budget slices then parsed; converges when all values are large", () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;

  let bootCallback = null;
  globalThis.setTimeout = (callback, _delay) => {
    bootCallback = callback;
    return 1;
  };
  globalThis.clearTimeout = () => {};

  const idleCallbacks = [];
  const now = Date.now();

  // Build values exceeding SWEEP_LARGE_VALUE_DEFER_BYTES with a valid updatedAt.
  const largePayload = "x".repeat(SWEEP_LARGE_VALUE_DEFER_BYTES + 1);
  const largeStaleValue = JSON.stringify({
    updatedAt: now - 14 * DAY_MS,
    payload: largePayload,
  });
  assert.ok(
    largeStaleValue.length > SWEEP_LARGE_VALUE_DEFER_BYTES,
    "test value must exceed the defer threshold",
  );

  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:large-a", largeStaleValue],
    ["buzz-channel-messages.v1:large-b", largeStaleValue],
  ]);

  installWindow(localStorage, {
    requestIdleCallback(callback, _options) {
      idleCallbacks.push(callback);
      return idleCallbacks.length;
    },
    cancelIdleCallback() {},
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    bootCallback();
    assert.equal(idleCallbacks.length, 1, "first slice scheduled after boot");

    // Slice 0 (zero budget): large-a is deferred (not yet parsed).
    idleCallbacks[0]({ timeRemaining: () => 0 });
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:large-a"),
      null,
      "large-a must not be removed after first slice (deferred)",
    );
    assert.ok(
      idleCallbacks.length >= 2,
      "second slice scheduled after deferral",
    );

    // Fire remaining slices with zero budget until sweep converges.
    // Worst-case: 2 defer slices (one per key) + 2 parse slices = 4 total.
    const MAX_SLICES = 20;
    let sliceCount = 1; // slice 0 already fired
    while (sliceCount < MAX_SLICES && idleCallbacks.length > sliceCount) {
      idleCallbacks[sliceCount]({ timeRemaining: () => 0 });
      sliceCount++;
    }

    // Both large stale entries must be removed after convergence.
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:large-a"),
      null,
      "large-a must be removed after convergence",
    );
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:large-b"),
      null,
      "large-b must be removed after convergence",
    );
    // Sweep converged in a bounded number of slices (≤ 2 * number of keys).
    assert.ok(
      sliceCount <= MAX_SLICES,
      `sweep did not converge within ${MAX_SLICES} slices`,
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});
