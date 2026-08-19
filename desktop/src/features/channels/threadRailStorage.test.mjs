import assert from "node:assert/strict";
import test from "node:test";

import {
  addThreadRailPin,
  DEFAULT_THREAD_RAIL_STORE,
  MAX_THREAD_RAIL_PINS,
  parseThreadRailStore,
  readThreadRailStore,
  removeThreadRailPin,
  updateThreadRailExpandedReplyIds,
  updateThreadRailPinAnchor,
  threadRailStorageKey,
  toggleThreadRailCollapsed,
  writeThreadRailStore,
} from "./threadRailStorage.ts";

if (typeof globalThis.window === "undefined") {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    },
  };
}

const SCOPE = { pubkey: "identity-a", relayUrl: "https://relay.example" };
const PIN_A = {
  channelId: "channel-a",
  rootId: "root-a",
  channelName: "general",
  rootExcerpt: "A useful thread",
  pinnedAt: 100,
};
const PIN_B = { channelId: "channel-b", rootId: "root-b", pinnedAt: 200 };

test("threadRailStorageKey scopes pins to normalized relay and identity", () => {
  assert.equal(
    threadRailStorageKey(SCOPE),
    "buzz-thread-rail.v1:https%3A%2F%2Frelay.example:identity-a",
  );
  assert.notEqual(
    threadRailStorageKey(SCOPE),
    threadRailStorageKey({ ...SCOPE, pubkey: "identity-b" }),
  );
  assert.equal(
    threadRailStorageKey(SCOPE),
    threadRailStorageKey({ ...SCOPE, relayUrl: "https://RELAY.example/" }),
  );
});

test("pinning is idempotent per channel and canonical root", () => {
  const once = addThreadRailPin(DEFAULT_THREAD_RAIL_STORE, PIN_A);
  const twice = addThreadRailPin(once, { ...PIN_A, pinnedAt: 999 });
  assert.equal(once.pins.length, 1);
  assert.equal(twice, once);
});

test("pins with the same root in different channels remain distinct", () => {
  const once = addThreadRailPin(DEFAULT_THREAD_RAIL_STORE, PIN_A);
  const twice = addThreadRailPin(once, { ...PIN_A, channelId: "channel-b" });
  assert.equal(twice.pins.length, 2);
});

test("updating a pin return anchor preserves canonical identity and order", () => {
  const store = {
    collapsed: false,
    pins: [PIN_A, PIN_B],
    version: 1,
  };

  assert.deepEqual(updateThreadRailPinAnchor(store, PIN_A, "nested-reply-a"), {
    collapsed: false,
    pins: [{ ...PIN_A, returnAnchorId: "nested-reply-a" }, PIN_B],
    version: 1,
  });
  assert.equal(
    updateThreadRailPinAnchor(store, PIN_A, "nested-reply-a").pins.length,
    2,
  );
  assert.equal(updateThreadRailPinAnchor(store, PIN_A, ""), store);
  assert.equal(
    updateThreadRailPinAnchor(
      store,
      { ...PIN_A, rootId: "missing" },
      "nested-reply-a",
    ),
    store,
  );
});

test("updates a pinned thread's expanded branches with a bounded stable local set", () => {
  const store = {
    collapsed: false,
    pins: [PIN_A, PIN_B],
    version: 1,
  };

  assert.deepEqual(
    updateThreadRailExpandedReplyIds(store, PIN_A, [
      "branch-a",
      "branch-a",
      "branch-b",
      "",
    ]),
    {
      collapsed: false,
      pins: [{ ...PIN_A, expandedReplyIds: ["branch-a", "branch-b"] }, PIN_B],
      version: 1,
    },
  );
});

test("unpin removes only the exact local rail entry and keeps collapse preference", () => {
  const store = {
    collapsed: true,
    pins: [PIN_A, PIN_B],
    version: 1,
  };
  assert.deepEqual(removeThreadRailPin(store, PIN_A), {
    collapsed: true,
    pins: [PIN_B],
    version: 1,
  });
});

test("collapse preference toggles without changing pins", () => {
  const store = addThreadRailPin(DEFAULT_THREAD_RAIL_STORE, PIN_A);
  assert.deepEqual(toggleThreadRailCollapsed(store), {
    collapsed: true,
    pins: [PIN_A],
    version: 1,
  });
});

test("parseThreadRailStore rejects malformed payloads while retaining valid pins", () => {
  assert.equal(parseThreadRailStore(null), null);
  assert.equal(parseThreadRailStore({ version: 2, pins: [] }), null);
  assert.deepEqual(
    parseThreadRailStore({
      version: 1,
      collapsed: true,
      pins: [
        { ...PIN_A, returnAnchorId: "nested-reply-a" },
        { channelId: 3, rootId: "bad" },
        { channelId: "c", rootId: "" },
        { ...PIN_B, returnAnchorId: 3 },
      ],
    }),
    {
      version: 1,
      collapsed: true,
      pins: [{ ...PIN_A, returnAnchorId: "nested-reply-a" }],
    },
  );
});

test("read/write round-trip persists collapse and isolates each identity and relay", () => {
  const written = {
    ...addThreadRailPin(DEFAULT_THREAD_RAIL_STORE, PIN_A),
    collapsed: true,
  };
  assert.equal(writeThreadRailStore(SCOPE, written), true);
  assert.deepEqual(readThreadRailStore(SCOPE), written);
  assert.deepEqual(
    readThreadRailStore({ ...SCOPE, pubkey: "identity-b" }),
    DEFAULT_THREAD_RAIL_STORE,
  );
  assert.deepEqual(
    readThreadRailStore({ ...SCOPE, relayUrl: "https://other-relay.example" }),
    DEFAULT_THREAD_RAIL_STORE,
  );
  window.localStorage.setItem(
    threadRailStorageKey({ ...SCOPE, pubkey: "broken" }),
    "{",
  );
  assert.deepEqual(
    readThreadRailStore({ ...SCOPE, pubkey: "broken" }),
    DEFAULT_THREAD_RAIL_STORE,
  );
});

test("read normalizes duplicate persisted pins by exact channel and root", () => {
  window.localStorage.setItem(
    threadRailStorageKey({ ...SCOPE, pubkey: "duplicate-pins" }),
    JSON.stringify({
      version: 1,
      collapsed: false,
      pins: [PIN_A, { ...PIN_A, pinnedAt: 999 }, PIN_B],
    }),
  );
  assert.deepEqual(
    readThreadRailStore({ ...SCOPE, pubkey: "duplicate-pins" }),
    {
      version: 1,
      collapsed: false,
      pins: [PIN_A, PIN_B],
    },
  );
});

test("pin collection and persisted labels stay bounded", () => {
  let store = DEFAULT_THREAD_RAIL_STORE;
  for (let index = 0; index < MAX_THREAD_RAIL_PINS + 5; index += 1) {
    store = addThreadRailPin(store, {
      channelId: `channel-${index}`,
      rootId: `root-${index}`,
      channelName: "c".repeat(200),
      rootExcerpt: "e".repeat(800),
      pinnedAt: index,
    });
  }

  assert.equal(store.pins.length, MAX_THREAD_RAIL_PINS);
  assert.equal(store.pins[0].rootId, "root-5");
  assert.equal(store.pins.at(-1).channelName.length, 128);
  assert.equal(store.pins.at(-1).rootExcerpt.length, 512);
});

test("parse rejects oversized durable identifiers", () => {
  const parsed = parseThreadRailStore({
    version: 1,
    collapsed: false,
    pins: [PIN_A, { ...PIN_B, rootId: "r".repeat(257) }],
  });
  assert.deepEqual(parsed?.pins, [PIN_A]);
});

test("write failure leaves the caller-owned in-memory store usable", () => {
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("storage unavailable");
  };
  try {
    assert.equal(
      writeThreadRailStore(
        SCOPE,
        addThreadRailPin(DEFAULT_THREAD_RAIL_STORE, PIN_A),
      ),
      false,
    );
  } finally {
    window.localStorage.setItem = original;
  }
});
