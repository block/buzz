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

test("pruneStaleContexts keeps a marker just read on an old message", () => {
  // The user marks a 30-day-old message read today. The marker carries the
  // MESSAGE's timestamp, so a value-ranked horizon drops it on the same write.
  const oldMessage = `msg:${"e".repeat(64)}`;
  const contexts = new Map([
    [oldMessage, NOW - READ_STATE_HORIZON_SECONDS * 4],
  ]);
  const readJustNow = new Map([[oldMessage, NOW]]);

  assert.equal(
    pruneStaleContexts(contexts, NOW).has(oldMessage),
    false,
    "value-ranked horizon drops it (documents the old behavior)",
  );
  assert.equal(
    pruneStaleContexts(contexts, NOW, readJustNow).has(oldMessage),
    true,
    "read-recency horizon keeps it",
  );
});

test("pruneStaleContexts cap evicts least recently read, not oldest message", () => {
  const contexts = new Map();
  const recency = new Map();
  const total = LOCAL_MAX_PRUNABLE_CONTEXTS + 1;
  // Newest messages first (i=0 newest), all read a long time ago...
  for (let i = 0; i < total - 1; i++) {
    const key = `msg:${String(i).padStart(64, "0")}`;
    contexts.set(key, NOW - i);
    recency.set(key, NOW - READ_STATE_HORIZON_SECONDS / 2);
  }
  // ...and one old message read just now, which must survive the cap.
  const justRead = `msg:${"f".repeat(64)}`;
  contexts.set(justRead, NOW - READ_STATE_HORIZON_SECONDS + 60);
  recency.set(justRead, NOW);

  const valueRanked = pruneStaleContexts(contexts, NOW);
  assert.equal(valueRanked.size, LOCAL_MAX_PRUNABLE_CONTEXTS);
  assert.equal(
    valueRanked.has(justRead),
    false,
    "value-ranked cap evicts it (documents the old behavior)",
  );

  const recencyRanked = pruneStaleContexts(contexts, NOW, recency);
  assert.equal(recencyRanked.size, LOCAL_MAX_PRUNABLE_CONTEXTS);
  assert.equal(recencyRanked.has(justRead), true, "read-recency cap keeps it");
});

test("writeStoredReadState keeps a marker just read on an old message", () => {
  installLocalStorage();
  const pubkey = "c".repeat(64);
  const nowSeconds = Math.floor(Date.now() / 1_000);
  const oldMessage = `msg:${"a".repeat(64)}`;

  writeStoredReadState(
    pubkey,
    new Map([[oldMessage, nowSeconds - READ_STATE_HORIZON_SECONDS * 4]]),
    new Set([oldMessage]),
    new Map([[oldMessage, nowSeconds]]),
  );

  const state = JSON.parse(
    window.localStorage.getItem(localReadStateKey(pubkey)),
  );
  assert.deepEqual(Object.keys(state), [oldMessage]);
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
