import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_THREAD_PIN_STORE,
  MAX_THREAD_PIN_ENTRIES,
  mergeThreadPinStores,
  parseThreadPinPayload,
  partitionByPin,
  pinnedRoots,
  togglePin,
} from "./threadPins.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);

function store(roots = {}) {
  return { version: 1, roots };
}

test("togglePin pins, then unpins as a tombstone, normalizing the root", () => {
  const pinned = togglePin(EMPTY_THREAD_PIN_STORE, A.toUpperCase(), 100);
  assert.deepEqual(pinned, store({ [A]: { pinned: true, updatedAt: 100 } }));
  assert.deepEqual(
    togglePin(pinned, A, 200),
    store({ [A]: { pinned: false, updatedAt: 200 } }),
  );
  assert.deepEqual([...pinnedRoots(togglePin(pinned, A, 200))], []);
});

test("togglePin advances updatedAt even when the clock went backwards", () => {
  const pinned = togglePin(EMPTY_THREAD_PIN_STORE, A, 500);
  const unpinned = togglePin(pinned, A, 100);
  assert.deepEqual(unpinned.roots[A], { pinned: false, updatedAt: 501 });
});

test("the cap evicts tombstones before pins, then the oldest pins", () => {
  let current = EMPTY_THREAD_PIN_STORE;
  for (let i = 0; i < MAX_THREAD_PIN_ENTRIES; i++) {
    current = togglePin(current, i.toString(16).padStart(64, "0"), 1_000 + i);
  }
  // Unpin the newest pin: it becomes the only tombstone.
  const newest = (MAX_THREAD_PIN_ENTRIES - 1).toString(16).padStart(64, "0");
  current = togglePin(current, newest, 5_000);
  current = togglePin(current, C, 10_000);
  assert.equal(Object.keys(current.roots).length, MAX_THREAD_PIN_ENTRIES);
  assert.ok(!(newest in current.roots), "the tombstone is evicted first");
  assert.ok(current.roots[C].pinned);

  current = togglePin(current, B, 11_000);
  assert.ok(
    !("0".repeat(64) in current.roots),
    "then the oldest pin is evicted",
  );
});

test("merge takes the newer entry per root, from either side", () => {
  const local = store({
    [A]: { pinned: true, updatedAt: 10 },
    [B]: { pinned: true, updatedAt: 10 },
  });
  const remote = store({
    [A]: { pinned: false, updatedAt: 20 },
    [B]: { pinned: false, updatedAt: 5 },
    [C]: { pinned: true, updatedAt: 1 },
  });
  assert.deepEqual(
    mergeThreadPinStores(local, remote),
    store({
      [A]: { pinned: false, updatedAt: 20 },
      [B]: { pinned: true, updatedAt: 10 },
      [C]: { pinned: true, updatedAt: 1 },
    }),
  );
});

test("an unpin on one device is not resurrected by another's stale pin", () => {
  const deviceOne = togglePin(EMPTY_THREAD_PIN_STORE, A, 100);
  const deviceTwo = togglePin(deviceOne, A, 200);
  assert.deepEqual(
    [...pinnedRoots(mergeThreadPinStores(deviceOne, deviceTwo))],
    [],
  );
  assert.deepEqual(
    [...pinnedRoots(mergeThreadPinStores(deviceTwo, deviceOne))],
    [],
  );
});

test("an exact-timestamp conflict resolves to unpinned", () => {
  const merged = mergeThreadPinStores(
    store({ [A]: { pinned: true, updatedAt: 7 } }),
    store({ [A]: { pinned: false, updatedAt: 7 } }),
  );
  assert.deepEqual(merged.roots[A], { pinned: false, updatedAt: 7 });
});

test("partitionByPin keeps activity order within each group", () => {
  const entries = [{ rootId: A }, { rootId: B.toUpperCase() }, { rootId: C }];
  const { pinned, unpinned } = partitionByPin(entries, new Set([C, B]));
  assert.deepEqual(
    pinned.map((entry) => entry.rootId),
    [B.toUpperCase(), C],
  );
  assert.deepEqual(
    unpinned.map((entry) => entry.rootId),
    [A],
  );
});

test("parseThreadPinPayload rejects malformed payloads and bad entries", () => {
  assert.equal(parseThreadPinPayload(null), null);
  assert.equal(parseThreadPinPayload([A]), null);
  assert.equal(parseThreadPinPayload({ version: 2, roots: {} }), null);
  assert.equal(parseThreadPinPayload({ version: 1, roots: [] }), null);
  assert.deepEqual(
    parseThreadPinPayload({
      version: 1,
      roots: {
        [A]: { pinned: true, updatedAt: 5 },
        short: { pinned: true, updatedAt: 5 },
        [A.toUpperCase()]: { pinned: true, updatedAt: 5 },
        [B]: { pinned: "yes", updatedAt: 5 },
        [C]: { pinned: true, updatedAt: -1 },
      },
    }),
    store({ [A]: { pinned: true, updatedAt: 5 } }),
  );
});
