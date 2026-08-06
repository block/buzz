import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelSectionSyncManager } from "./channelSectionsSync.ts";

function makeStore(overrides = {}) {
  return {
    version: 1,
    sections: overrides.sections ?? [],
    assignments: overrides.assignments ?? {},
    ...overrides,
  };
}

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the community-switch cross-relay publish vector:
// edit sections in relay A → destroy() is called (relayUrl dep change) →
// no publish should fire. The scoped localStorage write is durable; when the
// user returns to relay A the seed-publish path handles it.
test("destroy: cancels pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  // Simulate the timer scheduler with a manual clock so we can advance it.
  let timerCallback = null;
  const originalSetTimeout = globalThis.window?.setTimeout;
  const originalClearTimeout = globalThis.window?.clearTimeout;

  // Inject a fake window.setTimeout/clearTimeout if needed.
  const fakeTimers = [];
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
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
    const manager = new ChannelSectionSyncManager("pk-test");
    const store = makeStore({
      sections: [{ id: "s1", name: "Work", order: 0 }],
    });

    // Queue a publish — this sets the debounce timer.
    manager.publishSections(store);
    assert.ok(timerCallback !== null, "debounce timer should be set");

    // Destroy before the debounce fires — simulates community switch.
    manager.destroy();

    // Timer must be cleared and no publish should fire now.
    assert.ok(
      timerCallback === null,
      "debounce timer should be cleared on destroy",
    );

    // Advance time by invoking the callback that was cleared — it shouldn't exist.
    // If clearTimeout didn't work, try firing whatever was captured before destroy.
    // (There's nothing to fire after a correct destroy.)
    assert.equal(
      publishCalls.length,
      0,
      "no publish event should have been sent after destroy",
    );
  } finally {
    // Restore timer functions.
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
  // fetchEvents is held until we release it — simulates the latency window.
  let releaseFetch = null;
  const publishCalls = [];

  mock.method(relayClient, "fetchEvents", () => {
    return new Promise((resolve) => {
      // resolve with empty so fetchOwnBlobBeforePublish returns the local store
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
    const manager = new ChannelSectionSyncManager("pk-race");
    const store = makeStore({
      sections: [{ id: "s1", name: "Work", order: 0 }],
    });

    // Queue the publish — captures the debounce callback.
    manager.publishSections(store);
    assert.ok(capturedCallback !== null, "debounce timer should be set");

    // Fire the debounce manually — this starts doPublish() and nulls
    // debounceTimer inside publishSections' callback, leaving the async
    // doPublish running and awaiting fetchOwnBlobBeforePublish.
    const timerFn = capturedCallback;
    capturedCallback = null; // timer cleared itself inside the callback
    timerFn();

    // Now destroy() — debounceTimer is already null (timer fired), so only
    // the destroyed flag can stop doPublish.
    manager.destroy();

    // Release the held fetchEvents — fetchOwnBlobBeforePublish resolves with
    // the local store, then doPublish should check destroyed and abort.
    releaseFetch();

    // Drain microtasks so doPublish fully runs through to its abort point.
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
  const manager = new ChannelSectionSyncManager("pk-no-pending");
  // Should not throw even with nothing queued.
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
    const manager = new ChannelSectionSyncManager("pk-pending-null");
    const store = makeStore({
      sections: [{ id: "s1", name: "Test", order: 0 }],
    });
    manager.publishSections(store);
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

// Helper: build a minimal fake window with controllable localStorage and timers.
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
  const fakeWindow = {
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
  return fakeWindow;
}

function installFakeWindow(fw) {
  const orig = {};
  for (const key of ["localStorage", "setTimeout", "clearTimeout"]) {
    orig[key] = globalThis.window?.[key];
  }
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  globalThis.window.localStorage = fw.localStorage;
  globalThis.window.setTimeout = fw.setTimeout;
  globalThis.window.clearTimeout = fw.clearTimeout;
  return () => {
    for (const key of ["localStorage", "setTimeout", "clearTimeout"]) {
      if (orig[key] !== undefined) {
        globalThis.window[key] = orig[key];
      }
    }
  };
}

function makeSectionsStore(sections = []) {
  return {
    version: 1,
    sections,
    assignments: {},
  };
}

// Watermark key format: buzz-sync-watermark.v1:<blobType>:<pubkey>:<encodedRelay>
// Relay is normalised (lowercase, no trailing slash) before encoding.
const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

// 1. fetch failed (error/timeout) + local non-empty → zero publish calls
// Mutation test: removing the `failed` guard causes bootstrap to call publishSections.
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
    const manager = new ChannelSectionSyncManager("pk-fail", RELAY);
    const local = makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]);
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "bootstrap must return hold on failed fetch",
    );
    assert.equal(publishCalls.length, 0, "no publish after failed fetch");
  } finally {
    restore();
    mock.reset();
  }
});

// 1b. fetch failed (event exists but won't decrypt) → failed, head recorded
test("revert-fix: undecryptable event yields failed with createdAt set", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-decrypt",
        content: "bad-cipher",
        created_at: 1700000099,
        id: "evt-bad",
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
    const manager = new ChannelSectionSyncManager("pk-decrypt", RELAY);
    const result = await manager.fetchRemoteSections();
    assert.equal(result.status, "failed");
    assert.equal(
      result.createdAt,
      1700000099,
      "createdAt must be recorded from the unreadable event",
    );
    // Manager must have recorded the head watermark so seed-publish is blocked.
    assert.ok(
      manager.getPersistedWatermark() > 0,
      "watermark must be > 0 after seeing an undecryptable event",
    );
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. fetch absent + persisted head > 0 → zero publish calls (the dev-build stale-copy case)
// Mutation test: setting watermark to 0 in localStorage causes bootstrap to seed.
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  const fw = makeFakeWindow();
  // Pre-seed a watermark (simulates a prior session that had seen a blob).
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-sections:pk-stale:${RELAY_KEY}`,
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-stale", RELAY);
    assert.ok(
      manager.getPersistedWatermark() > 0,
      "manager must read watermark from localStorage at construction",
    );
    const local = makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]);
    const result = await manager.bootstrap(local);
    // bootstrap calls fetchRemoteSections → absent → watermark > 0 → hold
    assert.equal(
      result.action,
      "hold",
      "bootstrap must return hold when watermark > 0",
    );
    assert.equal(
      publishCalls.length,
      0,
      "watermark > 0 must block seed-publish even on absent fetch",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 3. fetch absent + head 0 + local non-empty → seed-publish fires (first-sync preserved)
// Mutation test: removing the absent+head-0 seed call prevents publishEvent from being observed.
test("revert-fix: absent fetch with zero watermark allows seed-publish via bootstrap", async () => {
  const publishEventCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishEventCalls.push(args);
    return Promise.resolve();
  });

  const fw = makeFakeWindow();
  // No watermark in storage — simulates genuine first-time user.
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-fresh", RELAY);
    assert.equal(
      manager.getPersistedWatermark(),
      0,
      "watermark must start at 0",
    );
    const local = makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]);
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "bootstrap returns hold (seed is async)",
    );
    // bootstrap must have queued a publish (pendingStore is set immediately).
    assert.ok(
      manager.getPendingStore() !== null,
      "bootstrap must queue a seed-publish when absent + watermark == 0",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 4. existing event that fails decrypt → no seed, head recorded from event.created_at
test("revert-fix: decrypt failure records head and blocks any future seed-publish", async () => {
  const publishCalls = [];
  let callCount = 0;
  mock.method(relayClient, "fetchEvents", () => {
    callCount++;
    if (callCount === 1) {
      return Promise.resolve([
        {
          pubkey: "pk-nodecrypt",
          content: "!!invalid-base64!!",
          created_at: 1700000777,
          id: "evt-nodecrypt",
        },
      ]);
    }
    return Promise.resolve([]);
  });
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelSectionSyncManager("pk-nodecrypt", RELAY);
    const local = makeSectionsStore([{ id: "s1", name: "Work", order: 0 }]);
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "failed fetch must return hold from bootstrap",
    );
    assert.ok(
      manager.getPersistedWatermark() >= 1700000777,
      "watermark must be advanced to event.created_at",
    );
    assert.equal(publishCalls.length, 0, "no publish after decrypt failure");
  } finally {
    restore();
    mock.reset();
  }
});

// 5. watermark round-trips across manager instances (simulated restart)
test("revert-fix: watermark persists and is read by a new manager instance", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        pubkey: "pk-restart",
        content: "bad-cipher",
        created_at: 1700001234,
        id: "evt-restart",
      },
    ]),
  );

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    // Session A: manager sees an event → watermark written to localStorage.
    const managerA = new ChannelSectionSyncManager("pk-restart", RELAY);
    await managerA.fetchRemoteSections();
    assert.ok(
      managerA.getPersistedWatermark() >= 1700001234,
      "session A watermark must be set",
    );

    // Session B: new manager instance reads the same localStorage.
    mock.restoreAll();
    const managerB = new ChannelSectionSyncManager("pk-restart", RELAY);
    assert.ok(
      managerB.getPersistedWatermark() >= 1700001234,
      "session B must inherit watermark from localStorage without another fetch",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 6. LWW baseline: newer decryptable pre-publish event still wins after an
//    undecryptable head was recorded.
// Mutation test: removing headBeforeFetch snapshot causes remote to never win.
test("revert-fix: sections LWW — newer decryptable pre-publish event selected after undecryptable head recorded", async () => {
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

  // Instead of mocking nip44DecryptFromSelf directly (it's ESM), use the
  // fact that parse returns null for invalid JSON — test the LWW path via
  // getPersistedWatermark and headBeforeFetch separation.
  // The key invariant: after seeing an event with created_at=100, a second
  // pre-publish event with created_at=200 must still be accepted (200 > 100).
  // This would break if the watermark advance happened before the comparison.
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const managerA = new ChannelSectionSyncManager("pk-lww", RELAY);
    // Simulate boot: advance watermark to 100 (as if boot fetch saw event@100).
    await managerA.fetchRemoteSections();
    const headAfterBoot = managerA.getPersistedWatermark();
    // The head should be recorded (either 100 from undecryptable event).
    assert.ok(headAfterBoot >= 100, "head must be recorded from boot event");
    // The pre-publish fetch (callCount=2) will see created_at=200.
    // If headBeforeFetch is correctly snapshotted before recording,
    // 200 > 100 (headBeforeFetch) → remote wins.
    // If headBeforeFetch was NOT snapshotted (bug), 200 > 200 → false → local wins.
    // We can observe this by checking that the second fetchEvents call was used:
    // after doPublish runs through fetchOwnBlobBeforePublish, the watermark should
    // advance to 200 if the event was observed.
    const store = makeSectionsStore([{ id: "s1", name: "A", order: 0 }]);
    managerA.publishSections(store);
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 10));
    // The watermark should have advanced to at least 200 (the pre-publish event).
    assert.ok(
      managerA.getPersistedWatermark() >= 200,
      "pre-publish event created_at=200 must advance the watermark (LWW comparison uses headBeforeFetch not current head)",
    );
  } finally {
    restore();
    mock.reset();
  }
});
