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
    const manager = new ChannelSortSyncManager("pk-test", "wss://r.test");
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
    const manager = new ChannelSortSyncManager("pk-race", "wss://r.test");
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
  const manager = new ChannelSortSyncManager("pk-no-pending", "wss://r.test");
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
    const manager = new ChannelSortSyncManager("pk-pending-null", "wss://r.test");
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

// Watermark key: buzz-sync-watermark.v1:channel-sort:<pubkey>:<encodedRelay>
const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

// 1. fetch failed (error/timeout) + local non-empty → hold, zero publish calls
// Mutation: removing the failed guard causes bootstrap to call publishSortPrefs → pendingStore set.
test("revert-fix: fetch failed (error) does not trigger seed-publish via bootstrap", async () => {
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
    const manager = new ChannelSortSyncManager("pk-fail", RELAY);
    const local = makeStore({ channels: "recent" });
    const result = await manager.bootstrap(local);
    assert.equal(result.action, "hold", "bootstrap must return hold on failed fetch");
    assert.equal(
      manager.getPendingStore(),
      null,
      "pendingStore must be null after failed fetch — no seed was queued",
    );
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. absent + persisted head > 0 → hold, zero publish calls (the dev-build stale-copy case)
// Mutation: setting watermark to 0 in localStorage causes bootstrap to seed.
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-sort:pk-stale:${RELAY_KEY}`,
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-stale", RELAY);
    assert.ok(manager.getPersistedWatermark() > 0);
    const local = makeStore({ channels: "recent" });
    const result = await manager.bootstrap(local);
    assert.equal(result.action, "hold", "bootstrap must return hold when watermark > 0");
    assert.equal(
      manager.getPendingStore(),
      null,
      "pendingStore must be null when watermark > 0 — no seed was queued",
    );
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + head 0 + local non-empty → seed-publish queued (first-sync preserved)
// Mutation: removing the absent+head-0 seed call leaves pendingStore null.
test("revert-fix: absent fetch with zero watermark seeds via bootstrap (first-sync preserved)", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-fresh", RELAY);
    assert.equal(manager.getPersistedWatermark(), 0, "watermark must start at 0");
    const local = makeStore({ channels: "recent" });
    const result = await manager.bootstrap(local);
    assert.equal(result.action, "hold", "bootstrap returns hold (seed is async)");
    assert.ok(
      manager.getPendingStore() !== null,
      "bootstrap must queue a seed-publish when absent + watermark == 0 + local non-empty",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 4. LWW baseline: newer decryptable pre-publish event still wins after an
//    undecryptable head was recorded.
// Mutation: removing headBeforeFetch snapshot causes remote to never win.
test("revert-fix: sort LWW — newer decryptable pre-publish event selected after undecryptable head recorded", async () => {
  // Boot fetch: undecryptable event, created_at=100 → head recorded to 100.
  // Pre-publish fetch: decryptable event, created_at=200 → should win (200 > 100).
  let callCount = 0;
  mock.method(relayClient, "fetchEvents", () => {
    callCount++;
    return Promise.resolve([
      {
        pubkey: "pk-lww",
        content: callCount === 1 ? "bad-cipher" : "good-cipher",
        created_at: callCount === 1 ? 100 : 200,
        id: `evt-${callCount}`,
      },
    ]);
  });
  mock.method(relayClient, "publishEvent", () => Promise.resolve());

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-lww", RELAY);
    // Boot fetch: sees event@100 (bad-cipher), records head to 100.
    await manager.fetchRemoteSortPrefs();
    assert.ok(manager.getPersistedWatermark() >= 100, "head must be recorded from boot event");
    // Queue a publish — triggers doPublish which calls fetchOwnBlobBeforePublish.
    const store = makeStore({ channels: "recent" });
    manager.publishSortPrefs(store);
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 10));
    // The pre-publish fetch (callCount=2) sees created_at=200.
    // If headBeforeFetch is correctly snapshotted, 200 > 100 → remote wins → watermark advances to 200.
    // If NOT snapshotted (bug), 200 > 200 → false → local wins → watermark stays at 100.
    assert.ok(
      manager.getPersistedWatermark() >= 200,
      "pre-publish event created_at=200 must advance the watermark (LWW comparison uses headBeforeFetch not current head)",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 5. live-sub: undecryptable event on live path records head before decrypt
// Mutation: removing recordRemoteHead before decrypt in the live callback
// leaves watermark at 0 after a live event.
test("revert-fix: undecryptable live event advances watermark before decrypt attempt", async () => {
  let liveCallback = null;
  mock.method(relayClient, "subscribeLive", (_filter, onEvent) => {
    liveCallback = onEvent;
    return Promise.resolve(async () => {});
  });

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSortSyncManager("pk-live", RELAY);
    assert.equal(manager.getPersistedWatermark(), 0, "watermark starts at 0");

    await manager.subscribeToSortPrefs(() => {});
    assert.ok(liveCallback !== null, "subscribeLive must have captured the callback");

    // Deliver an undecryptable event via the live callback.
    liveCallback({
      pubkey: "pk-live",
      content: "!bad-cipher!",
      created_at: 1700005555,
      id: "live-evt-1",
    });

    // Drain microtasks so the async decryptAndParse resolves.
    await new Promise((r) => setTimeout(r, 0));

    assert.ok(
      manager.getPersistedWatermark() >= 1700005555,
      "live undecryptable event must advance the watermark before decrypt is attempted",
    );
  } finally {
    restore();
    mock.reset();
  }
});
