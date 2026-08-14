import assert from "node:assert/strict";
import test from "node:test";

import {
  pruneStaleContexts,
  readStoredReadState,
  writeStoredReadState,
} from "./readStateStorage.ts";
import {
  LOCAL_MAX_PRUNABLE_CONTEXTS,
  READ_STATE_HORIZON_SECONDS,
  localPublishableContextKey,
  localReadStateKey,
  localSourceCreatedAtKey,
} from "./readStateFormat.ts";

function makeLocalStorage() {
  const store = new Map();
  return {
    get size() {
      return store.size;
    },
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
  };
}

function installLocalStorage() {
  const ls = makeLocalStorage();
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  globalThis.window.localStorage = ls;
  globalThis.localStorage = ls;
  return ls;
}

const NOW = 1_750_000_000;

test("pruneStaleContexts drops msg/thread markers older than horizon", () => {
  const cutoff = NOW - READ_STATE_HORIZON_SECONDS;
  const contexts = new Map([
    ["channel-1", cutoff - 999_999],
    [`thread:${"a".repeat(64)}`, cutoff - 1],
    [`thread:${"b".repeat(64)}`, cutoff + 1],
    [`msg:${"c".repeat(64)}`, cutoff - 1],
    [`msg:${"d".repeat(64)}`, cutoff + 1],
  ]);

  const pruned = pruneStaleContexts(contexts, NOW);

  assert.equal(pruned.has("channel-1"), true, "channel keys never pruned");
  assert.equal(pruned.has(`thread:${"a".repeat(64)}`), false);
  assert.equal(pruned.has(`thread:${"b".repeat(64)}`), true);
  assert.equal(pruned.has(`msg:${"c".repeat(64)}`), false);
  assert.equal(pruned.has(`msg:${"d".repeat(64)}`), true);
});

test("pruneStaleContexts caps within-horizon prunable entries, newest kept", () => {
  const contexts = new Map();
  const total = LOCAL_MAX_PRUNABLE_CONTEXTS + 50;
  for (let i = 0; i < total; i++) {
    contexts.set(`msg:${String(i).padStart(64, "0")}`, NOW - i);
  }

  const pruned = pruneStaleContexts(contexts, NOW);

  assert.equal(pruned.size, LOCAL_MAX_PRUNABLE_CONTEXTS);
  // Newest (i=0) survives; oldest (i=total-1) evicted.
  assert.equal(pruned.has(`msg:${String(0).padStart(64, "0")}`), true);
  assert.equal(pruned.has(`msg:${String(total - 1).padStart(64, "0")}`), false);
});

test("writeStoredReadState prunes all three keys consistently", () => {
  installLocalStorage();
  const pubkey = "f".repeat(64);
  const staleThread = `thread:${"a".repeat(64)}`;
  const freshThread = `thread:${"b".repeat(64)}`;
  const nowSeconds = Math.floor(Date.now() / 1_000);
  const stale = nowSeconds - READ_STATE_HORIZON_SECONDS - 10;

  writeStoredReadState(
    pubkey,
    new Map([
      ["channel-1", stale],
      [staleThread, stale],
      [freshThread, nowSeconds],
    ]),
    new Set(["channel-1", staleThread, freshThread]),
    new Map([
      ["channel-1", stale],
      [staleThread, stale],
      [freshThread, nowSeconds],
    ]),
  );

  const state = JSON.parse(
    window.localStorage.getItem(localReadStateKey(pubkey)),
  );
  assert.deepEqual(Object.keys(state).sort(), ["channel-1", freshThread]);

  const publishable = JSON.parse(
    window.localStorage.getItem(localPublishableContextKey(pubkey)),
  );
  assert.deepEqual(publishable.sort(), ["channel-1", freshThread]);

  const sourceCreatedAt = JSON.parse(
    window.localStorage.getItem(localSourceCreatedAtKey(pubkey)),
  );
  assert.deepEqual(Object.keys(sourceCreatedAt).sort(), [
    "channel-1",
    freshThread,
  ]);
});

test("writeStoredReadState round-trips through readStoredReadState", () => {
  installLocalStorage();
  const pubkey = "e".repeat(64);
  const nowSeconds = Math.floor(Date.now() / 1_000);

  writeStoredReadState(
    pubkey,
    new Map([["channel-9", nowSeconds]]),
    new Set(["channel-9"]),
    new Map([["channel-9", nowSeconds]]),
  );

  const stored = readStoredReadState(pubkey);
  assert.equal(stored.contexts.get("channel-9"), nowSeconds);
  assert.equal(stored.publishableContextIds.has("channel-9"), true);
  assert.equal(stored.contextSourceCreatedAt.get("channel-9"), nowSeconds);
});

test("writeStoredReadState survives a throwing localStorage.setItem", () => {
  const ls = installLocalStorage();
  ls.setItem = () => {
    throw new Error("QuotaExceededError");
  };
  const pubkey = "d".repeat(64);
  const nowSeconds = Math.floor(Date.now() / 1_000);

  assert.doesNotThrow(() => {
    writeStoredReadState(
      pubkey,
      new Map([["channel-1", nowSeconds]]),
      new Set(["channel-1"]),
      new Map([["channel-1", nowSeconds]]),
    );
  });
});
