import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelManualOrderSyncManager } from "./channelManualOrderSync.ts";

function makeStore(groups = {}) {
  return { version: 1, groups, manualGroups: [] };
}

function installFakeTimers() {
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  const originalSetTimeout = globalThis.window.setTimeout;
  const originalClearTimeout = globalThis.window.clearTimeout;
  let callback = null;
  let nextId = 1;
  globalThis.window.setTimeout = (fn) => {
    callback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = () => {
    callback = null;
  };
  return {
    getCallback: () => callback,
    restore() {
      globalThis.window.setTimeout = originalSetTimeout;
      globalThis.window.clearTimeout = originalClearTimeout;
    },
  };
}

test("destroy cancels a pending manual-order publish", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const timers = installFakeTimers();

  try {
    const manager = new ChannelManualOrderSyncManager("pk-test");
    manager.publish(makeStore({ channels: ["b", "a"] }));
    assert.ok(timers.getCallback());
    manager.destroy();
    assert.equal(timers.getCallback(), null);
    assert.equal(manager.getPendingStore(), null);
    assert.equal(publishCalls.length, 0);
  } finally {
    timers.restore();
    mock.reset();
  }
});

test("destroy aborts an in-flight manual-order publish", async () => {
  let releaseFetch;
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
    const manager = new ChannelManualOrderSyncManager("pk-race");
    manager.publish(makeStore({ channels: ["a"] }));
    const callback = timers.getCallback();
    assert.ok(callback);
    callback();
    manager.destroy();
    releaseFetch();
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(publishCalls.length, 0);
  } finally {
    timers.restore();
    mock.reset();
  }
});

test("destroy is safe without a pending manual-order publish", () => {
  const manager = new ChannelManualOrderSyncManager("pk-empty");
  assert.doesNotThrow(() => manager.destroy());
});

test("discarding a pending publish clears both timer and stale reconnect state", () => {
  const timers = installFakeTimers();
  try {
    const manager = new ChannelManualOrderSyncManager("pk-discard");
    manager.publish(makeStore({ channels: ["b", "a"] }));
    assert.ok(timers.getCallback());
    assert.ok(manager.getPendingStore());
    manager.discardPendingPublish();
    assert.equal(timers.getCallback(), null);
    assert.equal(manager.getPendingStore(), null);
  } finally {
    timers.restore();
  }
});

test("a remote update applies when no explicit local edit is pending", () => {
  const manager = new ChannelManualOrderSyncManager("pk-remote-first");
  assert.equal(
    manager.shouldApplyRemote({
      store: makeStore({ channels: ["a", "b"] }),
      createdAt: 10,
      eventId: "remote-a",
    }),
    true,
  );
});

test("a pending local edit deterministically defers a newer remote update", () => {
  const timers = installFakeTimers();
  try {
    const manager = new ChannelManualOrderSyncManager("pk-local-first");
    const local = makeStore({ channels: ["b", "a"] });
    manager.publish(local);
    assert.equal(
      manager.shouldApplyRemote({
        store: makeStore({ channels: ["a", "b"] }),
        createdAt: 20,
        eventId: "remote-b",
      }),
      false,
    );
    assert.deepEqual(manager.getPendingStore(), local);
    assert.ok(timers.getCallback());
  } finally {
    timers.restore();
  }
});

test("initial fetch and reconnect cannot replace a pending local edit", () => {
  const timers = installFakeTimers();
  try {
    const manager = new ChannelManualOrderSyncManager("pk-reconnect");
    const local = makeStore({ channels: ["c", "a", "b"] });
    manager.publish(local);
    for (const [createdAt, eventId] of [
      [30, "initial-fetch"],
      [40, "reconnect-fetch"],
    ]) {
      assert.equal(
        manager.shouldApplyRemote({
          store: makeStore({ channels: ["a", "b", "c"] }),
          createdAt,
          eventId,
        }),
        false,
      );
      assert.deepEqual(manager.getPendingStore(), local);
    }
  } finally {
    timers.restore();
  }
});
