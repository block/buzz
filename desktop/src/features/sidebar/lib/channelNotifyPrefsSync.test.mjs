import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelNotifyPrefsSyncManager } from "./channelNotifyPrefsSync.ts";

// The merge and publish-dedup rules the manager applies are pure functions
// (mergeStores / storesEqual) covered in channelNotifyPrefsStorage.test.mjs;
// these tests cover the manager's own scheduling and teardown behavior, which
// is where the cross-relay publish bugs (#1556) live.

function makeStore(channels = {}) {
  return { version: 1, channels };
}

function installFakeTimers() {
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const original = {
    setTimeout: globalThis.window.setTimeout,
    clearTimeout: globalThis.window.clearTimeout,
  };
  const state = { callback: null, nextId: 1 };
  globalThis.window.setTimeout = (fn) => {
    state.callback = fn;
    return state.nextId++;
  };
  globalThis.window.clearTimeout = () => {
    state.callback = null;
  };
  return {
    state,
    restore: () => {
      globalThis.window.setTimeout = original.setTimeout;
      globalThis.window.clearTimeout = original.clearTimeout;
    },
  };
}

test("publishPrefs: debounces and exposes the pending store", () => {
  const timers = installFakeTimers();
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-debounce");
    const store = makeStore({ c: { level: "mute", updatedAt: 1 } });
    manager.publishPrefs(store);
    assert.ok(timers.state.callback !== null, "debounce timer should be set");
    assert.deepEqual(manager.getPendingStore(), store);
  } finally {
    timers.restore();
  }
});

// Regression guard for the community-switch cross-relay publish vector (#1556):
// a level change on relay A followed by destroy() (relayUrl dep change) must not
// publish. The relay-scoped localStorage write is durable and the hook
// re-publishes when the user returns to relay A.
test("destroy: cancels the pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const timers = installFakeTimers();
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-destroy");
    manager.publishPrefs(makeStore({ c: { level: "mentions", updatedAt: 1 } }));
    manager.destroy();
    assert.equal(timers.state.callback, null, "timer must be cleared");
    assert.equal(
      manager.getPendingStore(),
      null,
      "pendingStore must be cleared",
    );
    assert.equal(publishCalls.length, 0, "nothing may be published on destroy");
  } finally {
    timers.restore();
    mock.reset();
  }
});

test("destroy: aborts an in-flight publish after the own-blob fetch resolves", async () => {
  let releaseFetch = null;
  const publishCalls = [];
  mock.method(
    relayClient,
    "fetchEvents",
    () =>
      new Promise((resolve) => {
        releaseFetch = () => resolve([]);
      }),
  );
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const timers = installFakeTimers();
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-race");
    manager.publishPrefs(makeStore({ c: { level: "mute", updatedAt: 1 } }));
    const timerFn = timers.state.callback;
    timers.state.callback = null; // the timer clears itself when it fires
    timerFn();

    manager.destroy();
    releaseFetch();
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.equal(
      publishCalls.length,
      0,
      "publishEvent must not run after destroy even once the timer fired",
    );
  } finally {
    timers.restore();
    mock.reset();
  }
});

test("destroy: is safe with no pending publish", () => {
  const manager = new ChannelNotifyPrefsSyncManager("pk-idle");
  assert.doesNotThrow(() => manager.destroy());
});

test("fetchRemotePrefs: null when the relay returns nothing", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-empty");
    assert.equal(await manager.fetchRemotePrefs(), null);
  } finally {
    mock.reset();
  }
});

test("fetchRemotePrefs: ignores events authored by anyone else", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        id: "evt",
        pubkey: "someone-else",
        created_at: 10,
        kind: 30078,
        tags: [["d", "channel-notify-prefs"]],
        content: "ciphertext",
        sig: "sig",
      },
    ]),
  );
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-mismatch");
    assert.equal(await manager.fetchRemotePrefs(), null);
  } finally {
    mock.reset();
  }
});

test("fetchRemotePrefs: swallows relay errors", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("relay down")),
  );
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-error");
    assert.equal(await manager.fetchRemotePrefs(), null);
  } finally {
    mock.reset();
  }
});

test("fetchRemotePrefs: queries kind 30078 scoped to the notify-prefs d-tag", async () => {
  const filters = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    filters.push(filter);
    return Promise.resolve([]);
  });
  try {
    const manager = new ChannelNotifyPrefsSyncManager("pk-filter");
    await manager.fetchRemotePrefs();
    assert.deepEqual(filters, [
      {
        kinds: [30078],
        authors: ["pk-filter"],
        "#d": ["channel-notify-prefs"],
        limit: 1,
      },
    ]);
  } finally {
    mock.reset();
  }
});
