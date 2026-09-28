import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieOwnerStorageKey,
  readOwnerScopedState,
  writeOwnerScopedState,
} from "./bestieOwnerScope.ts";

const SCOPE = {
  ownerPubkey: "B".repeat(64),
  relayUrl: "wss://Example.COM/relay",
};

test("owner storage key omits agent segment", () => {
  const key = bestieOwnerStorageKey("buzz-test.v1", SCOPE);
  assert.equal(key, `buzz-test.v1:wss://example.com/relay:${"b".repeat(64)}`);
});

test("readOwnerScopedState migrates and clears legacy agent keys", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      key: (index) => [...memory.keys()][index] ?? null,
      get length() {
        return memory.size;
      },
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  const ownerKey = bestieOwnerStorageKey("buzz-test.v1", SCOPE);
  const legacyA = `${ownerKey}:${"a".repeat(64)}`;
  const legacyC = `${ownerKey}:${"c".repeat(64)}`;
  memory.set(legacyA, JSON.stringify({ items: [{ id: "1" }], version: 1 }));
  memory.set(legacyC, JSON.stringify({ items: [{ id: "2" }], version: 1 }));

  const state = readOwnerScopedState({
    empty: () => ({ items: [], version: 1 }),
    isEmpty: (s) => s.items.length === 0,
    merge: (into, from) => ({
      items: [...into.items, ...from.items],
      version: 1,
    }),
    parse: (value) => {
      if (!value || typeof value !== "object") return null;
      return value;
    },
    prefix: "buzz-test.v1",
    scope: SCOPE,
  });

  assert.equal(state.items.length, 2);
  assert.ok(memory.has(ownerKey));
  assert.equal(memory.has(legacyA), false);
  assert.equal(memory.has(legacyC), false);

  writeOwnerScopedState("buzz-test.v1", SCOPE, { items: [{ id: "x" }], version: 1 });
  assert.equal(JSON.parse(memory.get(ownerKey)).items[0].id, "x");
});

test("intentional empty owner key is not remigrated from legacy", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      key: (index) => [...memory.keys()][index] ?? null,
      get length() {
        return memory.size;
      },
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  const ownerKey = bestieOwnerStorageKey("buzz-test.v1", SCOPE);
  const legacyA = `${ownerKey}:${"a".repeat(64)}`;
  memory.set(ownerKey, JSON.stringify({ items: [], version: 1 }));
  memory.set(legacyA, JSON.stringify({ items: [{ id: "zombie" }], version: 1 }));

  const state = readOwnerScopedState({
    empty: () => ({ items: [], version: 1 }),
    isEmpty: (s) => s.items.length === 0,
    merge: (into, from) => ({
      items: [...into.items, ...from.items],
      version: 1,
    }),
    parse: (value) => {
      if (!value || typeof value !== "object") return null;
      return value;
    },
    prefix: "buzz-test.v1",
    scope: SCOPE,
  });

  assert.deepEqual(state.items, []);
  // Write clears leftover legacy keys.
  writeOwnerScopedState("buzz-test.v1", SCOPE, { items: [], version: 1 });
  assert.equal(memory.has(legacyA), false);
});
