import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelMuteSyncManager } from "./channelMutesSync.ts";

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
    const manager = new ChannelMuteSyncManager("pk-test");
    const store = makeStore({ ch1: { muted: true, updatedAt: 100 } });
    manager.publishMutes(store);
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
    const manager = new ChannelMuteSyncManager("pk-race");
    const store = makeStore({ ch1: { muted: true, updatedAt: 100 } });
    manager.publishMutes(store);
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
    const manager = new ChannelMuteSyncManager("pk-no-pending");
    assert.doesNotThrow(() => manager.destroy());
  } finally {
    restore();
  }
});

// ─── Boot seed-publish guard (the revert-fix regression suite) ────────────────

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
    const manager = new ChannelMuteSyncManager("pk-fail");
    const result = await manager.fetchRemoteMutes();
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
    const manager = new ChannelMuteSyncManager("pk-dc");
    const result = await manager.fetchRemoteMutes();
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
    "buzz-sync-watermark.v1:channel-mutes:pk-stale",
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-stale");
    assert.ok(manager.getPersistedWatermark() > 0);
    const result = await manager.fetchRemoteMutes();
    assert.equal(result.status, "absent");
    if (result.status === "absent" && manager.getPersistedWatermark() === 0) {
      manager.publishMutes(makeStore({ ch1: { muted: true, updatedAt: 1 } }));
      fw._fireTimer();
      await new Promise((r) => setTimeout(r, 0));
    }
    assert.equal(publishCalls.length, 0);
  } finally {
    restore();
    mock.reset();
  }
});

// 3. absent + head 0 → seed allowed
test("revert-fix: absent fetch with zero watermark allows seed-publish", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelMuteSyncManager("pk-fresh");
    assert.equal(manager.getPersistedWatermark(), 0);
    const result = await manager.fetchRemoteMutes();
    assert.equal(result.status, "absent");
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
    const manager = new ChannelMuteSyncManager("pk-nd");
    const result = await manager.fetchRemoteMutes();
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
    const managerA = new ChannelMuteSyncManager("pk-restart");
    await managerA.fetchRemoteMutes();
    assert.ok(managerA.getPersistedWatermark() >= 1700001234);
    mock.restoreAll();
    const managerB = new ChannelMuteSyncManager("pk-restart");
    assert.ok(managerB.getPersistedWatermark() >= 1700001234);
  } finally {
    restore();
    mock.reset();
  }
});
