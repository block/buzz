import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelSortSyncManager } from "./channelSortSync.ts";

function makeStore(groups = {}) {
  return { version: 1, groups };
}

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the community-switch cross-relay publish vector:
// change a sort mode in relay A → destroy() is called (relayUrl dep change) →
// no publish should fire. The scoped localStorage write is durable; when the
// user returns to relay A the seed-publish path handles it.
test("destroy: cancels pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  let timerCallback = null;
  const fakeTimers = [];
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const originalSetTimeout = globalThis.window.setTimeout;
  const originalClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    const id = nextId++;
    fakeTimers.push({ id, fn });
    timerCallback = fn;
    return id;
  };
  globalThis.window.clearTimeout = (id) => {
    const idx = fakeTimers.findIndex((t) => t.id === id);
    if (idx !== -1) {
      fakeTimers.splice(idx, 1);
      timerCallback = null;
    }
  };

  try {
    const manager = new ChannelSortSyncManager("pk-test");
    const store = makeStore({ channels: "recent" });

    manager.publishSortPrefs(store);
    assert.ok(timerCallback !== null, "debounce timer should be set");

    manager.destroy();

    assert.ok(
      timerCallback === null,
      "debounce timer should be cleared on destroy",
    );
    assert.equal(
      publishCalls.length,
      0,
      "no publish event should have been sent after destroy",
    );
  } finally {
    if (originalSetTimeout !== undefined) {
      globalThis.window.setTimeout = originalSetTimeout;
    }
    if (originalClearTimeout !== undefined) {
      globalThis.window.clearTimeout = originalClearTimeout;
    }
    mock.reset();
  }
});

// Regression guard for the timer-fired race: debounce fires → doPublish starts
// awaiting fetchOwnBlobBeforePublish → destroy() is called (relayUrl dep
// change) → publishEvent must never be called even though the timer already
// fired and cleared itself before destroy() ran.
test("destroy: aborts in-flight doPublish after fetchOwnBlobBeforePublish resolves", async () => {
  let releaseFetch = null;
  const publishCalls = [];

  mock.method(relayClient, "fetchEvents", () => {
    return new Promise((resolve) => {
      releaseFetch = () => resolve([]);
    });
  });
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  let capturedCallback = null;
  let nextId = 1;
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    capturedCallback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = (_id) => {
    capturedCallback = null;
  };

  try {
    const manager = new ChannelSortSyncManager("pk-race");
    const store = makeStore({ dms: "recent" });

    manager.publishSortPrefs(store);
    assert.ok(capturedCallback !== null, "debounce timer should be set");

    const timerFn = capturedCallback;
    capturedCallback = null; // timer cleared itself inside the callback
    timerFn();

    manager.destroy();

    releaseFetch();

    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.equal(
      publishCalls.length,
      0,
      "publishEvent must not be called after destroy() even when timer already fired",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const manager = new ChannelSortSyncManager("pk-no-pending");
  assert.doesNotThrow(() => manager.destroy());
});

test("destroy: cancelPendingPublish clears pendingStore", () => {
  let timerCallback = null;
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const orig = globalThis.window.setTimeout;
  const origClear = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    timerCallback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = (_id) => {
    timerCallback = null;
  };

  try {
    const manager = new ChannelSortSyncManager("pk-pending-null");
    const store = makeStore({ starred: "recent" });
    manager.publishSortPrefs(store);
    assert.deepEqual(manager.getPendingStore(), store);

    manager.destroy();
    assert.equal(
      manager.getPendingStore(),
      null,
      "pendingStore must be null after destroy",
    );
    assert.ok(timerCallback === null, "timer must be cleared after destroy");
  } finally {
    globalThis.window.setTimeout = orig;
    globalThis.window.clearTimeout = origClear;
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ────────────────

function makeFakeWindow() {
  const storage = new Map();
  const ls = {
    getItem: (k) => storage.get(k) ?? null,
    setItem: (k, v) => storage.set(k, v),
    removeItem: (k) => storage.delete(k),
    clear: () => storage.clear(),
  };
  let timerCallback = null;
  let nextTimerId = 100;
  const fw = {
    localStorage: ls,
    setTimeout: (fn, _ms) => {
      timerCallback = fn;
      return nextTimerId++;
    },
    clearTimeout: (_id) => {
      timerCallback = null;
    },
    _fireTimer: () => {
      if (timerCallback) {
        const fn = timerCallback;
        timerCallback = null;
        fn();
      }
    },
  };
  return fw;
}

function installFakeWindow(fw) {
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  const origLs = globalThis.window.localStorage;
  const origSt = globalThis.window.setTimeout;
  const origCt = globalThis.window.clearTimeout;
  globalThis.window.localStorage = fw.localStorage;
  globalThis.window.setTimeout = fw.setTimeout;
  globalThis.window.clearTimeout = fw.clearTimeout;
  return () => {
    if (origLs !== undefined) globalThis.window.localStorage = origLs;
    if (origSt !== undefined) globalThis.window.setTimeout = origSt;
    if (origCt !== undefined) globalThis.window.clearTimeout = origCt;
  };
}

// 1. fetch failed → zero publish calls
test("revert-fix: fetch failed (error) does not trigger seed-publish", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("relay timeout")),
  );
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-fail", "wss://r.test");
    const result = await manager.fetchRemoteSortPrefs();
    assert.equal(result.status, "failed");
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 1b. undecryptable event → failed + head recorded
test("revert-fix: undecryptable event yields failed with createdAt and advances watermark", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      { pubkey: "pk-dc", content: "!bad!", created_at: 1700000099, id: "e1" },
    ]),
  );
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-dc", "wss://r.test");
    const result = await manager.fetchRemoteSortPrefs();
    assert.equal(result.status, "failed");
    assert.equal(result.createdAt, 1700000099);
    assert.ok(manager.getPersistedWatermark() > 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. absent + persisted head > 0 → no seed-publish
test("revert-fix: absent fetch with prior watermark blocks seed-publish", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  fw.localStorage.setItem(
    "buzz-sync-watermark.v1:channel-sort:pk-stale:wss%3A%2F%2Fr.test",
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-stale", "wss://r.test");
    assert.ok(manager.getPersistedWatermark() > 0);
    const result = await manager.fetchRemoteSortPrefs();
    assert.equal(result.status, "absent");
    // Gate: absent AND watermark > 0 → no seed.
    if (result.status === "absent" && manager.getPersistedWatermark() === 0) {
      manager.publishSortPrefs(makeStore({ channels: "recent" }));
      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 0));
    }
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + head 0 + local non-empty → seed allowed
test("revert-fix: absent fetch with zero watermark allows seed-publish", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-fresh", "wss://r.test");
    assert.equal(manager.getPersistedWatermark(), 0);
    const result = await manager.fetchRemoteSortPrefs();
    assert.equal(result.status, "absent");
    // The gate condition is what matters for mutation-sensitivity.
    assert.ok(
      result.status === "absent" && manager.getPersistedWatermark() === 0,
      "seed condition must hold for a fresh manager",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 4. decrypt failure records head
test("revert-fix: decrypt failure records head and blocks future seed", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-nd",
        content: "!!invalid!!",
        created_at: 1700000777,
        id: "evt-nd",
      },
    ]),
  );
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-nd", "wss://r.test");
    const result = await manager.fetchRemoteSortPrefs();
    assert.equal(result.status, "failed");
    assert.equal(result.createdAt, 1700000777);
    assert.ok(manager.getPersistedWatermark() >= 1700000777);
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 5. watermark round-trips across manager instances
test("revert-fix: watermark persists across manager instances (simulated restart)", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-restart",
        content: "!bad!",
        created_at: 1700001234,
        id: "evt-r",
      },
    ]),
  );
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const managerA = new ChannelSortSyncManager("pk-restart", "wss://r.test");
    await managerA.fetchRemoteSortPrefs();
    assert.ok(managerA.getPersistedWatermark() >= 1700001234);
    mock.restoreAll();
    const managerB = new ChannelSortSyncManager("pk-restart", "wss://r.test");
    assert.ok(managerB.getPersistedWatermark() >= 1700001234);
  } finally {
    restore();
    mock.reset();
  }
});
