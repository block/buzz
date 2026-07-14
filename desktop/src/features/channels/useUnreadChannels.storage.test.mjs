import assert from "node:assert/strict";
import test from "node:test";

import {
  activityScopeKey,
  activityStorageKey,
  addThreadActivityItems,
  projectActivityForScope,
  readActivityFromStorage,
  writeActivityToStorage,
} from "./threadActivityStorage.ts";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

// Mock window.localStorage with a simple in-memory store.
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

// Helper to isolate each test's localStorage state.
function makeIsolatedStorage() {
  const store = new Map();
  const prev = globalThis.window.localStorage;
  globalThis.window.localStorage = {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
  };
  return {
    store,
    restore: () => {
      globalThis.window.localStorage = prev;
    },
  };
}

function makeItem(id, channelId = "channel-1", createdAt = 1) {
  return {
    id,
    kind: 9,
    pubkey: "author",
    content: "reply",
    createdAt,
    channelId,
    channelName: "general",
    tags: [
      ["h", channelId],
      ["e", "root-1", "", "root"],
    ],
  };
}

// ── activityStorageKey (production function) ─────────────────────────────────

test("activityStorageKey normalizes relay URL before embedding", () => {
  const key1 = activityStorageKey("pk1", "WSS://Relay.Example.Com/");
  const key2 = activityStorageKey("pk1", "wss://relay.example.com");
  assert.equal(key1, key2);
});

test("activityStorageKey produces different keys for different relays", () => {
  const keyA = activityStorageKey("pk1", "wss://relay-a.example.com");
  const keyB = activityStorageKey("pk1", "wss://relay-b.example.com");
  assert.notEqual(keyA, keyB);
});

test("activityStorageKey produces different keys for different pubkeys", () => {
  const key1 = activityStorageKey("pk1", "wss://relay.example.com");
  const key2 = activityStorageKey("pk2", "wss://relay.example.com");
  assert.notEqual(key1, key2);
});

test("activityStorageKey differs from legacy unscoped key", () => {
  const pubkey = "abc123";
  const relay = "wss://relay.example.com";
  const legacyKey = `buzz-thread-activity.v1:${pubkey}`;
  const scopedKey = activityStorageKey(pubkey, relay);
  assert.notEqual(legacyKey, scopedKey);
  assert.ok(
    scopedKey.includes(normalizeRelayUrl(relay)),
    "scoped key should contain normalized relay URL",
  );
});

// ── write/read round-trip using production functions ─────────────────────────

test("round-trip: items written for relay A are readable under relay A", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relayA = "wss://relay-a.example.com";
    const items = [makeItem("reply-a1", "channel-1", 1)];

    writeActivityToStorage(pubkey, relayA, items);
    const read = readActivityFromStorage(pubkey, relayA);

    assert.equal(read.length, 1);
    assert.equal(read[0].id, "reply-a1");
  } finally {
    isolated.restore();
  }
});

test("round-trip: items written for relay A are NOT readable under relay B", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relayA = "wss://relay-a.example.com";
    const relayB = "wss://relay-b.example.com";
    const items = [makeItem("reply-a1", "channel-1", 1)];

    writeActivityToStorage(pubkey, relayA, items);
    const read = readActivityFromStorage(pubkey, relayB);

    assert.deepEqual(read, []);
  } finally {
    isolated.restore();
  }
});

test("round-trip: A→B→A — A rows absent in B, A rows return on switch back", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relayA = "wss://relay-a.example.com";
    const relayB = "wss://relay-b.example.com";

    // Workspace A accumulates two activity rows.
    const itemsA = [
      makeItem("reply-a1", "channel-a1", 1),
      makeItem("reply-a2", "channel-a2", 2),
    ];
    writeActivityToStorage(pubkey, relayA, itemsA);

    // Workspace B has its own rows.
    const itemsB = [makeItem("reply-b1", "channel-b1", 3)];
    writeActivityToStorage(pubkey, relayB, itemsB);

    // While in workspace B, reading relay A gives B's rows (not A's).
    const inB = readActivityFromStorage(pubkey, relayB);
    assert.equal(inB.length, 1);
    assert.equal(inB[0].id, "reply-b1");

    // A's rows must not appear in B.
    assert.ok(
      !inB.some((item) => item.id === "reply-a1" || item.id === "reply-a2"),
      "relay A rows must not appear when reading relay B bucket",
    );

    // Switch back to A — A's persisted rows return.
    const backInA = readActivityFromStorage(pubkey, relayA);
    assert.equal(backInA.length, 2);
    assert.ok(backInA.some((item) => item.id === "reply-a1"));
    assert.ok(backInA.some((item) => item.id === "reply-a2"));
  } finally {
    isolated.restore();
  }
});

test("round-trip: trailing slash on relay URL collapses to same bucket", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relayWithSlash = "wss://relay.example.com/";
    const relayWithout = "wss://relay.example.com";
    const items = [makeItem("reply-1", "channel-1", 1)];

    writeActivityToStorage(pubkey, relayWithSlash, items);
    const read = readActivityFromStorage(pubkey, relayWithout);

    assert.equal(read.length, 1);
    assert.equal(read[0].id, "reply-1");
  } finally {
    isolated.restore();
  }
});

test("round-trip: corrupt JSON in storage returns empty array without throwing", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relay = "wss://relay.example.com";
    globalThis.window.localStorage.setItem(
      activityStorageKey(pubkey, relay),
      "not-valid-json{{{",
    );
    const read = readActivityFromStorage(pubkey, relay);
    assert.deepEqual(read, []);
  } finally {
    isolated.restore();
  }
});

test("round-trip: non-array JSON in storage returns empty array", () => {
  const isolated = makeIsolatedStorage();
  try {
    const pubkey = "pk1";
    const relay = "wss://relay.example.com";
    globalThis.window.localStorage.setItem(
      activityStorageKey(pubkey, relay),
      JSON.stringify({ not: "an array" }),
    );
    const read = readActivityFromStorage(pubkey, relay);
    assert.deepEqual(read, []);
  } finally {
    isolated.restore();
  }
});

// ── addThreadActivityItems ───────────────────────────────────────────────────

test("addThreadActivityItems deduplicates by id", () => {
  const existing = [makeItem("a", "ch", 1)];
  const { didAdd, items } = addThreadActivityItems(existing, [
    makeItem("a", "ch", 1),
  ]);
  assert.equal(didAdd, false);
  assert.equal(items.length, 1);
});

test("addThreadActivityItems merges new items sorted by createdAt", () => {
  const existing = [makeItem("a", "ch", 1), makeItem("c", "ch", 3)];
  const { didAdd, items } = addThreadActivityItems(existing, [
    makeItem("b", "ch", 2),
  ]);
  assert.equal(didAdd, true);
  assert.deepEqual(
    items.map((item) => item.id),
    ["a", "b", "c"],
  );
});

test("addThreadActivityItems caps at MAX_ACTIVITY_ITEMS (100) keeping newest", () => {
  const existing = Array.from({ length: 99 }, (_, i) =>
    makeItem(`old-${i}`, "ch", i + 1),
  );
  const incoming = [
    makeItem("new-100", "ch", 200),
    makeItem("new-101", "ch", 201),
  ];
  const { didAdd, items } = addThreadActivityItems(existing, incoming);
  assert.equal(didAdd, true);
  assert.equal(items.length, 100);
  assert.ok(items.some((item) => item.id === "new-100"));
  assert.ok(items.some((item) => item.id === "new-101"));
});

test("addThreadActivityItems returns didAdd false when all items are duplicates", () => {
  const existing = [makeItem("x", "ch", 1), makeItem("y", "ch", 2)];
  const { didAdd, items } = addThreadActivityItems(existing, [
    makeItem("x", "ch", 1),
    makeItem("y", "ch", 2),
  ]);
  assert.equal(didAdd, false);
  assert.equal(items, existing); // same reference
});

// ── activityScopeKey (scope-ref identity helper) ─────────────────────────────

test("activityScopeKey returns empty string when pubkey is null", () => {
  assert.equal(activityScopeKey(null, "wss://relay.example.com"), "");
});

test("activityScopeKey returns empty string when relayUrl is empty", () => {
  assert.equal(activityScopeKey("pk1", ""), "");
});

test("activityScopeKey normalizes relay URL", () => {
  const k1 = activityScopeKey("pk1", "WSS://Relay.Example.Com/");
  const k2 = activityScopeKey("pk1", "wss://relay.example.com");
  assert.equal(k1, k2);
});

test("activityScopeKey differs for different pubkeys", () => {
  const k1 = activityScopeKey("pk1", "wss://relay.example.com");
  const k2 = activityScopeKey("pk2", "wss://relay.example.com");
  assert.notEqual(k1, k2);
});

test("activityScopeKey differs for different relays", () => {
  const k1 = activityScopeKey("pk1", "wss://relay-a.example.com");
  const k2 = activityScopeKey("pk1", "wss://relay-b.example.com");
  assert.notEqual(k1, k2);
});

// ── scope-transition render fence (state-machine proof) ──────────────────────
//
// These tests model the hook's render fence without a React harness:
//   threadActivityScopeRef.current = scope at last effect commit
//   currentActivityScope            = scope derived this render
//   threadActivityRef.current       = in-memory items
//
// The fence: return threadActivityRef.current only when
//   threadActivityScopeRef.current === currentActivityScope
//
// This proves that A rows are hidden on the first B render (before the reset
// effect commits), and restored only when A is active again.

function makeScopeState({ pubkey, relayUrl, loadedItems = [] } = {}) {
  const loaded = loadedItems;
  // Simulate what the reset effect writes (executed after commit, not in render).
  const effectCommitScope = activityScopeKey(pubkey ?? null, relayUrl ?? "");
  return {
    threadActivityScopeRef: { current: effectCommitScope },
    threadActivityItems: loaded,
  };
}

function renderFence(
  threadActivityScopeRef,
  threadActivityItems,
  pubkey,
  relayUrl,
) {
  const currentScope = activityScopeKey(pubkey ?? null, relayUrl ?? "");
  return projectActivityForScope(
    threadActivityScopeRef.current,
    currentScope,
    threadActivityItems,
  );
}

test("scope-transition: A rows visible when scope matches A (steady state)", () => {
  const relayA = "wss://relay-a.example.com";
  const pubkey = "pk1";
  const itemsA = [{ id: "a1" }];
  const state = makeScopeState({
    pubkey,
    relayUrl: relayA,
    loadedItems: itemsA,
  });

  const visible = renderFence(
    state.threadActivityScopeRef,
    state.threadActivityItems,
    pubkey,
    relayA,
  );
  assert.deepEqual(visible, itemsA);
});

test("scope-transition: A rows hidden on first B render (scope mismatch before effect commits)", () => {
  const relayA = "wss://relay-a.example.com";
  const relayB = "wss://relay-b.example.com";
  const pubkey = "pk1";
  const itemsA = [{ id: "a1" }, { id: "a2" }];

  // State reflects A's committed scope (reset effect hasn't run for B yet).
  const state = makeScopeState({
    pubkey,
    relayUrl: relayA,
    loadedItems: itemsA,
  });

  // Render fires with B props before reset effect commits — fence returns [].
  const visible = renderFence(
    state.threadActivityScopeRef,
    state.threadActivityItems,
    pubkey,
    relayB,
  );
  assert.deepEqual(
    visible,
    [],
    "A rows must not be visible on the B transition render",
  );
});

test("scope-transition: B rows visible after B reset effect commits", () => {
  const relayB = "wss://relay-b.example.com";
  const pubkey = "pk1";
  const itemsB = [{ id: "b1" }];

  // Reset effect has now committed for B.
  const state = makeScopeState({
    pubkey,
    relayUrl: relayB,
    loadedItems: itemsB,
  });

  const visible = renderFence(
    state.threadActivityScopeRef,
    state.threadActivityItems,
    pubkey,
    relayB,
  );
  assert.deepEqual(visible, itemsB);
});

test("scope-transition: A→B→A — A rows return when A's reset effect commits again", () => {
  const relayA = "wss://relay-a.example.com";
  const relayB = "wss://relay-b.example.com";
  const pubkey = "pk1";
  const itemsA = [{ id: "a1" }, { id: "a2" }];
  const itemsB = [{ id: "b1" }];

  // Step 1: in A, scope and ref match A.
  const stateA = makeScopeState({
    pubkey,
    relayUrl: relayA,
    loadedItems: itemsA,
  });
  assert.deepEqual(
    renderFence(
      stateA.threadActivityScopeRef,
      stateA.threadActivityItems,
      pubkey,
      relayA,
    ),
    itemsA,
  );

  // Step 2: switch to B — render fires before effect commits (stale A scope in ref).
  assert.deepEqual(
    renderFence(
      stateA.threadActivityScopeRef,
      stateA.threadActivityItems,
      pubkey,
      relayB,
    ),
    [],
    "A rows must be hidden on B transition render",
  );

  // Step 3: B reset effect commits — now ref holds B scope and B items.
  const stateB = makeScopeState({
    pubkey,
    relayUrl: relayB,
    loadedItems: itemsB,
  });
  assert.deepEqual(
    renderFence(
      stateB.threadActivityScopeRef,
      stateB.threadActivityItems,
      pubkey,
      relayB,
    ),
    itemsB,
  );

  // Step 4: switch back to A — render fires before effect commits (stale B scope).
  assert.deepEqual(
    renderFence(
      stateB.threadActivityScopeRef,
      stateB.threadActivityItems,
      pubkey,
      relayA,
    ),
    [],
    "B rows must be hidden on A transition render",
  );

  // Step 5: A reset effect commits — A rows return.
  const stateA2 = makeScopeState({
    pubkey,
    relayUrl: relayA,
    loadedItems: itemsA,
  });
  assert.deepEqual(
    renderFence(
      stateA2.threadActivityScopeRef,
      stateA2.threadActivityItems,
      pubkey,
      relayA,
    ),
    itemsA,
    "A rows must return when A's reset effect commits again",
  );
});

test("scope-transition: empty pubkey produces empty fence regardless of relay", () => {
  // When currentActivityScope is "" (no pubkey), the hook never loads items
  // into threadActivityRef — the reset effect guards on normalizedPubkey.
  // So scope "" with empty items correctly returns [].
  const state = {
    threadActivityScopeRef: { current: "" },
    threadActivityItems: [],
  };
  const visible = renderFence(
    state.threadActivityScopeRef,
    state.threadActivityItems,
    null,
    "wss://relay.example.com",
  );
  assert.deepEqual(visible, []);
});

// Direct projectActivityForScope tests — these call the production helper
// without going through renderFence, ensuring the helper itself is correct.

test("projectActivityForScope: rejects empty currentScope even if loadedScope is also empty", () => {
  // The ref initializes to "" and writers can fire before the first reset
  // effect commits. "" === "" must never expose items.
  // Use a non-empty items array so the test actually proves rejection
  // (an empty input array would stay green even if the helper returned it).
  const items = [{ id: "x1" }, { id: "x2" }];
  assert.deepEqual(
    projectActivityForScope("", "", items),
    [],
    "empty currentScope must always return [] regardless of items content",
  );
});

test("projectActivityForScope: returns items when both scopes are equal and non-empty", () => {
  const items = [{ id: "x1" }];
  assert.deepEqual(
    projectActivityForScope(
      "pk:wss://relay.example.com/",
      "pk:wss://relay.example.com/",
      items,
    ),
    items,
  );
});

test("projectActivityForScope: returns [] on scope mismatch", () => {
  const items = [{ id: "x1" }];
  assert.deepEqual(
    projectActivityForScope(
      "pk:wss://relay-a.example.com/",
      "pk:wss://relay-b.example.com/",
      items,
    ),
    [],
  );
});
