import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelStarSyncManager } from "./channelStarsSync.ts";
import {
  makeFakeWindow,
  installFakeWindow,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.test";
const RELAY_KEY = encodeURIComponent(RELAY);

function makeStore(channels = {}) {
  return { version: 1, channels };
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
    manager.publishStars(makeStore({ ch1: { starred: true, updatedAt: 100 } }));
    manager.destroy();
    assert.equal(publishCalls.length, 0, "no publish after destroy");
    assert.equal(manager.getPendingStarStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy: aborts in-flight doPublish after fetchOwnBlobBeforePublish resolves", async () => {
  let releaseFetch = null;
  const publishCalls = [];
  mock.method(
    relayClient,
    "fetchEvents",
    () =>
      new Promise((res) => {
        releaseFetch = () => res([]);
      }),
  );
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-race", RELAY);
    manager.publishStars(makeStore({ ch1: { starred: true, updatedAt: 100 } }));
    fw._fireTimer();
    manager.destroy();
    releaseFetch();
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

// 1. fetch failed → hold, pendingStore null (mutation: remove failed guard → seed queued)
test("revert-fix: fetch failed (error) does not trigger seed-publish via bootstrap", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("relay timeout")),
  );
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-fail", RELAY);
    const result = await manager.bootstrap(
      makeStore({ ch1: { starred: true, updatedAt: 1 } }),
    );
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStarStore(), null);
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
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-stars:pk-stale:${RELAY_KEY}`,
    "1700000000",
  );
  const restore = installFakeWindow(fw);
  try {
    const manager = new ChannelStarSyncManager("pk-stale", RELAY);
    assert.ok(
      Number(
        fw.localStorage.getItem(
          `buzz-sync-watermark.v1:channel-stars:pk-stale:${RELAY_KEY}`,
        ) ?? "0",
      ) > 0,
    );
    const result = await manager.bootstrap(
      makeStore({ ch1: { starred: true, updatedAt: 1 } }),
    );
    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStarStore(), null);
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
    const manager = new ChannelStarSyncManager("pk-fresh", RELAY);
    assert.equal(
      fw.localStorage.getItem(
        `buzz-sync-watermark.v1:channel-stars:pk-fresh:${RELAY_KEY}`,
      ),
      null,
    );
    const result = await manager.bootstrap(
      makeStore({ ch1: { starred: true, updatedAt: 1 } }),
    );
    assert.equal(result.action, "hold");
    assert.ok(manager.getPendingStarStore() !== null);
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
  fw.localStorage.setItem(
    `buzz-sync-watermark.v1:channel-stars:pk-iso:${encodeURIComponent(relayA)}`,
    "1700000100",
  );
  const restore = installFakeWindow(fw);
  try {
    const managerB = new ChannelStarSyncManager("pk-iso", relayB);
    assert.equal(
      fw.localStorage.getItem(
        `buzz-sync-watermark.v1:channel-stars:pk-iso:${encodeURIComponent(relayB)}`,
      ),
      null,
      "relay B watermark must be independent of relay A head",
    );
    const result = await managerB.bootstrap(
      makeStore({ ch1: { starred: true, updatedAt: 1 } }),
    );
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
