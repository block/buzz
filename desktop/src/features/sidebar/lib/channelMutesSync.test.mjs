import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelMuteSyncManager } from "./channelMutesSync.ts";

const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

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
  return {
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
// mute a channel in relay A → destroy() called (relayUrl dep change) →
// no publish should fire.
test("destroy: cancels pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => { publishCalls.push(args); return Promise.resolve(); });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-test", RELAY);
    manager.publishMutes(makeStore({ ch1: { muted: true, updatedAt: 100 } }));
    manager.destroy();
    assert.equal(publishCalls.length, 0);
    assert.equal(manager.getPendingMuteStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: aborts in-flight doPublish after fetchOwnBlobBeforePublish resolves", async () => {
  let releaseFetch = null;
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => new Promise((res) => { releaseFetch = () => res([]); }));
  mock.method(relayClient, "publishEvent", (...args) => { publishCalls.push(args); return Promise.resolve(); });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-race", RELAY);
    manager.publishMutes(makeStore({ ch1: { muted: true, updatedAt: 100 } }));
    fw._fireTimer();
    manager.destroy();
    releaseFetch();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-no-pending", RELAY);
    assert.doesNotThrow(() => manager.destroy());
  } finally {
    restore();
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ─────────────────

// 1. fetch failed → hold, pendingStore null (mutation: remove failed guard → seed queued)
test("revert-fix: fetch failed (error) does not trigger seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.reject(new Error("relay timeout")));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-fail", RELAY);
    const result = await manager.bootstrap(makeStore({ ch1: { muted: true, updatedAt: 1 } }));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingMuteStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 2. absent + prior watermark → hold, pendingStore null (mutation: clear watermark → seed queued)
test("revert-fix: absent fetch with prior watermark blocks seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  fw.localStorage.setItem(`buzz-sync-watermark.v1:channel-mutes:pk-stale:${RELAY_KEY}`, "1700000000");
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-stale", RELAY);
    assert.ok(manager.getPersistedWatermark() > 0);
    const result = await manager.bootstrap(makeStore({ ch1: { muted: true, updatedAt: 1 } }));
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingMuteStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + zero watermark + non-empty → seed queued (mutation: remove seed call → pendingStore null)
test("revert-fix: absent fetch with zero watermark seeds via bootstrap (first-sync preserved)", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-fresh", RELAY);
    assert.equal(manager.getPersistedWatermark(), 0);
    const result = await manager.bootstrap(makeStore({ ch1: { muted: true, updatedAt: 1 } }));
    assert.equal(result.action, "hold");
    assert.ok(manager.getPendingMuteStore() !== null);
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
  fw.localStorage.setItem(`buzz-sync-watermark.v1:channel-mutes:pk-iso:${encodeURIComponent(relayA)}`, "1700000100");
  const restore = installFakeWindow(fw);
  try {
    const managerB = new ChannelMuteSyncManager("pk-iso", relayB);
    assert.equal(managerB.getPersistedWatermark(), 0, "relay B watermark must be independent of relay A head");
    const result = await managerB.bootstrap(makeStore({ ch1: { muted: true, updatedAt: 1 } }));
    assert.equal(result.action, "hold");
    assert.ok(managerB.getPendingMuteStore() !== null, "first-sync seed on relay B must not be blocked by relay A watermark");
  } finally {
    restore();
    mock.reset();
  }
});
