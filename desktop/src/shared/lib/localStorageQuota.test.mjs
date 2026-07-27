import assert from "node:assert/strict";
import test from "node:test";

import {
  recoverLocalStorageQuotaOnStartup,
  setLocalStorageItemWithRecovery,
} from "./localStorageQuota.ts";

function makeQuotaLocalStorage({ maxEntries }) {
  const store = new Map();
  return {
    store,
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem(key, value) {
      if (!store.has(key) && store.size >= maxEntries) {
        throw new Error("QuotaExceededError");
      }
      store.set(key, value);
    },
    removeItem: (key) => store.delete(key),
  };
}

function install(ls) {
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  globalThis.window.localStorage = ls;
  globalThis.localStorage = ls;
}

test("startup recovery removes disposable caches but preserves user state", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 5 });
  install(ls);
  ls.store.set("buzz-channel-messages.v1:relay:chan", "big");
  ls.store.set("buzz-channels.v1:relay", "big");
  ls.store.set("buzz-timeline-skeleton-shape.v1:chan", "small");
  ls.store.set("buzz-sidebar-skeleton-shape.v1:community:user", "small");
  ls.store.set("buzz-communities", "keep");

  recoverLocalStorageQuotaOnStartup();

  assert.equal(ls.getItem("buzz-channel-messages.v1:relay:chan"), null);
  assert.equal(ls.getItem("buzz-channels.v1:relay"), null);
  assert.equal(ls.getItem("buzz-timeline-skeleton-shape.v1:chan"), null);
  assert.equal(
    ls.getItem("buzz-sidebar-skeleton-shape.v1:community:user"),
    null,
  );
  assert.equal(ls.getItem("buzz-communities"), "keep");
  assert.equal(ls.getItem("buzz-local-storage-quota-recovery.v1"), "1");
});

test("healthy startup preserves disposable caches", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 10 });
  install(ls);
  ls.store.set("buzz-channel-messages.v1:relay:new", "snapshot");

  recoverLocalStorageQuotaOnStartup();

  assert.equal(ls.getItem("buzz-channel-messages.v1:relay:new"), "snapshot");
  assert.equal(ls.getItem("buzz-local-storage-quota-recovery.v1"), "1");
});

test("startup recovery does not remove namespace near misses", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 2 });
  install(ls);
  ls.store.set("buzz-channels.v10:durable", "keep");
  ls.store.set("buzz-channel-messages.v1-durable", "keep");

  recoverLocalStorageQuotaOnStartup();

  assert.equal(ls.getItem("buzz-channels.v10:durable"), "keep");
  assert.equal(ls.getItem("buzz-channel-messages.v1-durable"), "keep");
  assert.equal(ls.getItem("buzz-local-storage-quota-recovery.v1"), null);
});

test("startup recovery runs only once", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 10 });
  install(ls);

  recoverLocalStorageQuotaOnStartup();
  ls.store.set("buzz-channel-messages.v1:relay:new", "new snapshot");
  recoverLocalStorageQuotaOnStartup();

  assert.equal(
    ls.getItem("buzz-channel-messages.v1:relay:new"),
    "new snapshot",
  );
});

test("startup recovery retries after marker write fails", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 1 });
  install(ls);
  ls.store.set("buzz-communities", "keep");

  recoverLocalStorageQuotaOnStartup();
  assert.equal(ls.getItem("buzz-local-storage-quota-recovery.v1"), null);

  ls.store.delete("buzz-communities");
  ls.store.set("buzz-channel-messages.v1:relay:chan", "big");
  recoverLocalStorageQuotaOnStartup();

  assert.equal(ls.getItem("buzz-channel-messages.v1:relay:chan"), null);
  assert.equal(ls.getItem("buzz-local-storage-quota-recovery.v1"), "1");
});

test("global cache byte budget evicts only oldest entries needed", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 20 });
  install(ls);
  ls.store.set("buzz-communities", "keep");
  const snapshot = (updatedAt) =>
    JSON.stringify({ updatedAt, payload: "x".repeat(400_000) });
  const oldestKey = "buzz-channel-messages.v1:relay:oldest";
  const newerKey = "buzz-channels.v1:relay-newer";
  const newestKey = "buzz-channel-messages.v1:relay:newest";

  assert.equal(setLocalStorageItemWithRecovery(oldestKey, snapshot(1)), true);
  assert.equal(setLocalStorageItemWithRecovery(newerKey, snapshot(2)), true);
  assert.equal(setLocalStorageItemWithRecovery(newestKey, snapshot(3)), true);

  assert.equal(ls.getItem(oldestKey), null);
  assert.notEqual(ls.getItem(newerKey), null);
  assert.notEqual(ls.getItem(newestKey), null);
  assert.equal(ls.getItem("buzz-communities"), "keep");
});

test("global cache byte budget spans relays and preserves durable state", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 20 });
  install(ls);
  ls.store.set("buzz-communities", "keep");
  const largeSnapshot = "x".repeat(600_000);

  assert.equal(
    setLocalStorageItemWithRecovery(
      "buzz-channel-messages.v1:relay-one:chan",
      largeSnapshot,
    ),
    true,
  );
  assert.equal(
    setLocalStorageItemWithRecovery(
      "buzz-channel-messages.v1:relay-two:chan",
      largeSnapshot,
    ),
    true,
  );

  assert.equal(ls.getItem("buzz-channel-messages.v1:relay-one:chan"), null);
  assert.equal(
    ls.getItem("buzz-channel-messages.v1:relay-two:chan"),
    largeSnapshot,
  );
  assert.equal(ls.getItem("buzz-communities"), "keep");
});

test("rejects a single cache entry larger than the global byte budget", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 10 });
  install(ls);
  const key = "buzz-channel-messages.v1:relay:oversized";
  ls.store.set(key, "previous snapshot");

  assert.equal(
    setLocalStorageItemWithRecovery(key, "x".repeat(1_100_000)),
    false,
  );
  assert.equal(ls.getItem(key), null);
});

test("writes normally when under quota", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 10 });
  install(ls);
  assert.equal(setLocalStorageItemWithRecovery("k", "v"), true);
  assert.equal(ls.getItem("k"), "v");
});

test("evicts pure caches and retries on quota failure", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 2 });
  install(ls);
  ls.store.set("buzz-channel-messages.v1:relay:chan", "big");
  ls.store.set("buzz-channels.v1:relay", "big");

  assert.equal(setLocalStorageItemWithRecovery("k", "v"), true);
  assert.equal(ls.getItem("k"), "v");
  assert.equal(ls.getItem("buzz-channel-messages.v1:relay:chan"), null);
  assert.equal(ls.getItem("buzz-channels.v1:relay"), null);
});

test("returns false when eviction frees nothing", () => {
  const ls = makeQuotaLocalStorage({ maxEntries: 2 });
  install(ls);
  ls.store.set("buzz-workspaces", "keep");
  ls.store.set("buzz-active-workspace-id", "keep");

  assert.equal(setLocalStorageItemWithRecovery("k", "v"), false);
  assert.equal(ls.getItem("k"), null);
  assert.equal(ls.getItem("buzz-workspaces"), "keep");
});
