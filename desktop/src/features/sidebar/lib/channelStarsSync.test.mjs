import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelStarSyncManager } from "./channelStarsSync.ts";

// Relay URL used in all tests. Watermark key format:
//   buzz-sync-watermark.v1:channel-stars:<pubkey>:<encodeURIComponent(normalizeRelay(url))>
// normalizeRelay: trim + lowercase + strip trailing slash.
const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY); // "wss%3A%2F%2Fr.test"

function makeStore(channels = {}) {
  return { version: 1, channels };
}

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

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the community-switch cross-relay publish vector:
// star a channel in relay A → destroy() called (relayUrl dep change) →
// no publish should fire.
test("destroy: cancels pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-test", RELAY);
    const store = makeStore({ ch1: { starred: true, updatedAt: 100 } });

    manager.publishStars(store);
    assert.ok(fw.localStorage !== undefined, "window should be set up");

    manager.destroy();
    assert.equal(publishCalls.length, 0, "no publish after destroy");
    assert.equal(
      manager.getPendingStarStore(),
      null,
      "pendingStore must be null after destroy",
    );
  } finally {
    restore();
    mock.reset();
  }
});

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

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-race", RELAY);
    const store = makeStore({ ch1: { starred: true, updatedAt: 100 } });

    manager.publishStars(store);
    fw._fireTimer(); // fire debounce → doPublish starts

    manager.destroy();
    releaseFetch(); // fetchOwnBlobBeforePublish resolves
    await new Promise((r) => setTimeout(r, 0));

    assert.equal(
      publishCalls.length,
      0,
      "publishEvent must not be called after destroy",
    );
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-no-pending", RELAY);
    assert.doesNotThrow(() => manager.destroy());
  } finally {
    restore();
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ─────────────────
//
// All tests below drive the production bootstrap() path so that a regression
// in that code — not just a hook wiring change — causes a test failure.
// Mutation-sensitivity note: each guard is named in the comment before the test.

// 1. fetch failed (error/timeout) + local non-empty → zero publish calls
// Mutation: removing the `failed` guard causes bootstrap to call publishStars → pendingStore set.
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
    const manager = new ChannelStarSyncManager("pk-fail", RELAY);
    const local = makeStore({ ch1: { starred: true, updatedAt: 1 } });
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "bootstrap must return hold on failed fetch",
    );
    assert.equal(
      manager.getPendingStarStore(),
      null,
      "no pending publish after failed fetch",
    );
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. fetch absent + persisted head > 0 → zero publish calls (the dev-build stale-copy case)
// Mutation: setting watermark to 0 in localStorage causes bootstrap to seed.
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  // Pre-seed a watermark with the relay-scoped key (simulates a prior session).
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-stars:pk-stale:${RELAY_KEY}`,
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-stale", RELAY);
    assert.ok(
      manager.getPersistedWatermark() > 0,
      "manager must read relay-scoped watermark from localStorage at construction",
    );
    const local = makeStore({ ch1: { starred: true, updatedAt: 1 } });
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "bootstrap must return hold when watermark > 0",
    );
    assert.equal(
      manager.getPendingStarStore(),
      null,
      "watermark > 0 must block seed-publish even on absent fetch",
    );
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. fetch absent + head 0 + local non-empty → seed-publish fires (first-sync preserved)
// Mutation: removing the absent+head-0 seed call leaves pendingStore null.
test("revert-fix: absent fetch with zero watermark seeds via bootstrap (first-sync preserved)", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-fresh", RELAY);
    assert.equal(
      manager.getPersistedWatermark(),
      0,
      "watermark must start at 0",
    );
    const local = makeStore({ ch1: { starred: true, updatedAt: 1 } });
    const result = await manager.bootstrap(local);
    assert.equal(
      result.action,
      "hold",
      "bootstrap returns hold (seed is async)",
    );
    // bootstrap must have queued a publish via publishStars — pendingStore is set immediately.
    assert.ok(
      manager.getPendingStarStore() !== null,
      "bootstrap must queue a seed-publish when absent + watermark == 0 + local non-empty",
    );
  } finally {
    restore();
    mock.reset();
  }
});

// 4. relay-A / relay-B watermark isolation
// Mutation: using pubkey-only key (no relay) makes relay A's head suppress relay B's first-sync.
test("revert-fix: relay-A watermark does not suppress first-sync seed on relay-B", async () => {
  const relayA = "wss://a.relay.test";
  const relayB = "wss://b.relay.test";
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  // Simulate relay A having a prior head.
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-stars:pk-iso:${encodeURIComponent(relayA)}`,
    "1700000100",
  );
  const restore = installFakeWindow(fw);
  try {
    // Manager on relay B must start with watermark 0 despite relay A having one.
    const managerB = new ChannelStarSyncManager("pk-iso", relayB);
    assert.equal(
      managerB.getPersistedWatermark(),
      0,
      "relay B watermark must be independent of relay A head",
    );
    // And first-sync seed on relay B should be allowed.
    const local = makeStore({ ch1: { starred: true, updatedAt: 1 } });
    const result = await managerB.bootstrap(local);
    assert.equal(result.action, "hold");
    assert.ok(
      managerB.getPendingStarStore() !== null,
      "first-sync seed on relay B must not be blocked by relay A watermark",
    );
  } finally {
    restore();
    mock.reset();
  }
});
