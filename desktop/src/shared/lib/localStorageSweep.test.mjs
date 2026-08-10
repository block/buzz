import assert from "node:assert/strict";
import test from "node:test";

import {
  LOCAL_STORAGE_SWEEP_RULES,
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
  const originalDocument = globalThis.document;
  const originalWarn = console.warn;
  const warnings = [];
  console.warn = (...args) => warnings.push(args);
  globalThis.document = {
    addEventListener() {
      throw new Error("listener unavailable");
    },
    removeEventListener() {},
    visibilityState: "visible",
  };
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
    globalThis.document = originalDocument;
  }
});

test("scheduler uses a deferred timer when requestIdleCallback is unavailable", () => {
  const originalDocument = globalThis.document;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const documentTarget = new EventTarget();
  Object.defineProperty(documentTarget, "visibilityState", {
    value: "visible",
  });
  globalThis.document = documentTarget;

  const timeouts = new Map();
  let nextTimeoutId = 50;
  globalThis.setTimeout = (callback, delay) => {
    const id = nextTimeoutId++;
    timeouts.set(id, { callback, delay });
    return id;
  };
  globalThis.clearTimeout = (id) => timeouts.delete(id);

  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:startup", snapshot(0)],
  ]);
  installWindow(localStorage, {
    setInterval: () => 1,
    clearInterval() {},
  });

  const stop = startLocalStorageSweep();
  try {
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
    );
    const [{ callback, delay }] = timeouts.values();
    assert.equal(delay, 250);
    callback();
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
    );
  } finally {
    stop();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
    globalThis.document = originalDocument;
  }
  assert.equal(timeouts.size, 0);
});

test("scheduler sweeps after idle, on debounced visibility, and hourly", () => {
  const originalNow = Date.now;
  const originalDocument = globalThis.document;
  let now = 100 * DAY_MS;
  Date.now = () => now;

  const documentTarget = new EventTarget();
  Object.defineProperty(documentTarget, "visibilityState", {
    configurable: true,
    value: "visible",
    writable: true,
  });
  globalThis.document = documentTarget;

  const intervals = new Map();
  const idleCallbacks = new Map();
  const cancelledIdleCallbacks = [];
  let nextIntervalId = 1;
  let nextIdleId = 100;
  const localStorage = makeLocalStorage([
    ["buzz-channel-messages.v1:startup", snapshot(now - 14 * DAY_MS)],
  ]);
  installWindow(localStorage, {
    requestIdleCallback(callback, options) {
      const id = nextIdleId++;
      idleCallbacks.set(id, { callback, options });
      return id;
    },
    cancelIdleCallback(id) {
      cancelledIdleCallbacks.push(id);
      idleCallbacks.delete(id);
    },
    setInterval(callback, delay) {
      const id = nextIntervalId++;
      intervals.set(id, { callback, delay });
      return id;
    },
    clearInterval: (id) => intervals.delete(id),
  });

  let idleId;
  const stop = startLocalStorageSweep();
  try {
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
      "initial sweep must not run synchronously on the boot path",
    );
    assert.equal(idleCallbacks.size, 1);
    const idleEntry = idleCallbacks.entries().next().value;
    idleId = idleEntry[0];
    const { callback: idleCallback, options } = idleEntry[1];
    assert.equal(options.timeout, 1_500);
    idleCallback();
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:startup"),
      null,
    );
    assert.equal(intervals.size, 1);
    const [{ callback, delay }] = intervals.values();
    assert.equal(delay, 60 * 60 * 1_000);

    localStorage.setItem(
      "buzz-channel-messages.v1:visibility",
      snapshot(now - 14 * DAY_MS),
    );
    now += 60 * 1_000;
    documentTarget.dispatchEvent(new Event("visibilitychange"));
    assert.notEqual(
      localStorage.getItem("buzz-channel-messages.v1:visibility"),
      null,
    );

    now += 5 * 60 * 1_000;
    documentTarget.dispatchEvent(new Event("visibilitychange"));
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:visibility"),
      null,
    );

    localStorage.setItem(
      "buzz-channel-messages.v1:interval",
      snapshot(now - 14 * DAY_MS),
    );
    now += 60 * 60 * 1_000;
    callback();
    assert.equal(
      localStorage.getItem("buzz-channel-messages.v1:interval"),
      null,
    );
  } finally {
    stop();
    Date.now = originalNow;
    globalThis.document = originalDocument;
  }
  assert.equal(intervals.size, 0);
  assert.deepEqual(cancelledIdleCallbacks, [idleId]);
});
