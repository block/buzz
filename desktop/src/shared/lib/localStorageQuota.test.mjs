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
  const ls = makeQuotaLocalStorage({ maxEntries: 10 });
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
