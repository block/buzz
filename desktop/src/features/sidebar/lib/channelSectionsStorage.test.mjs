import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_STORE,
  parseChannelSectionPayload,
  readChannelSectionsStore,
  storageKey,
  stripOrphanedAssignments,
  writeChannelSectionsStore,
} from "./channelSectionsStorage.ts";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

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

function makeStore(overrides = {}) {
  return {
    version: 1,
    sections: overrides.sections ?? [{ id: "s1", name: "Test", order: 0 }],
    assignments: overrides.assignments ?? {},
    ...overrides,
  };
}

function makeSection(overrides = {}) {
  return { id: "s1", name: "Test", order: 0, ...overrides };
}

test("parseChannelSectionPayload: valid complete payload returns correct store", () => {
  const payload = {
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: { chan1: "s1" },
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result, {
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: { chan1: "s1" },
  });
});

test("parseChannelSectionPayload: null input returns null", () => {
  assert.equal(parseChannelSectionPayload(null), null);
});

test("parseChannelSectionPayload: non-object input returns null", () => {
  assert.equal(parseChannelSectionPayload("string"), null);
  assert.equal(parseChannelSectionPayload(42), null);
  assert.equal(parseChannelSectionPayload(true), null);
});

test("parseChannelSectionPayload: missing sections returns empty sections array", () => {
  const result = parseChannelSectionPayload({ assignments: {} });
  assert.deepEqual(result?.sections, []);
});

test("parseChannelSectionPayload: malformed section entries are filtered out", () => {
  const payload = {
    sections: [
      { id: 123, name: "Bad ID", order: 0 },
      { id: "s1", name: 456, order: 0 },
      { id: "s2", name: "Good", order: "not-a-number" },
      null,
      "string-entry",
    ],
    assignments: {},
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result?.sections, []);
});

test("parseChannelSectionPayload: valid sections with some invalid ones filters correctly", () => {
  const payload = {
    sections: [
      { id: "s1", name: "Valid", order: 0 },
      { id: 99, name: "Bad ID", order: 1 },
      { id: "s2", name: "Also Valid", order: 2 },
    ],
    assignments: {},
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result?.sections, [
    { id: "s1", name: "Valid", order: 0 },
    { id: "s2", name: "Also Valid", order: 2 },
  ]);
});

test("parseChannelSectionPayload: missing assignments returns empty assignments object", () => {
  const result = parseChannelSectionPayload({ sections: [] });
  assert.deepEqual(result?.assignments, {});
});

test("parseChannelSectionPayload: assignments with non-string values are filtered out", () => {
  const payload = {
    sections: [{ id: "s1", name: "Test", order: 0 }],
    assignments: { chan1: "s1", chan2: 42, chan3: null, chan4: true },
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result?.assignments, { chan1: "s1" });
});

test("parseChannelSectionPayload: orphaned assignments are stripped", () => {
  const payload = {
    sections: [{ id: "s1", name: "Exists", order: 0 }],
    assignments: { chan1: "s1", chan2: "missing-section" },
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result?.assignments, { chan1: "s1" });
});

test("stripOrphanedAssignments: store with no orphans returns same reference", () => {
  const store = makeStore({
    sections: [makeSection({ id: "s1" })],
    assignments: { chan1: "s1" },
  });
  assert.equal(stripOrphanedAssignments(store), store);
});

test("stripOrphanedAssignments: store with orphaned assignments returns new object without them", () => {
  const store = makeStore({
    sections: [makeSection({ id: "s1" })],
    assignments: { chan1: "s1", chan2: "ghost" },
  });
  const result = stripOrphanedAssignments(store);
  assert.notEqual(result, store);
  assert.deepEqual(result.assignments, { chan1: "s1" });
});

test("stripOrphanedAssignments: store with all valid assignments returns same reference", () => {
  const store = makeStore({
    sections: [
      makeSection({ id: "s1" }),
      makeSection({ id: "s2", name: "B", order: 1 }),
    ],
    assignments: { chan1: "s1", chan2: "s2" },
  });
  assert.equal(stripOrphanedAssignments(store), store);
});

test("stripOrphanedAssignments: empty store returns same reference", () => {
  const store = makeStore({ sections: [], assignments: {} });
  assert.equal(stripOrphanedAssignments(store), store);
});

test("writeChannelSectionsStore + readChannelSectionsStore: write then read returns same data", () => {
  const pubkey = "pk-roundtrip";
  const store = makeStore({
    sections: [makeSection({ id: "s1", name: "Work", order: 0 })],
    assignments: { chan1: "s1" },
  });
  const written = writeChannelSectionsStore(pubkey, store);
  assert.equal(written, true);
  const result = readChannelSectionsStore(pubkey);
  assert.deepEqual(result, store);
});

test("readChannelSectionsStore: non-existent key returns DEFAULT_STORE", () => {
  const result = readChannelSectionsStore("pk-does-not-exist-xyz");
  assert.deepEqual(result, DEFAULT_STORE);
});

test("readChannelSectionsStore: corrupt JSON returns DEFAULT_STORE", () => {
  const pubkey = "pk-corrupt";
  window.localStorage.setItem(storageKey(pubkey), "not-valid-json{{{");
  const result = readChannelSectionsStore(pubkey);
  assert.deepEqual(result, DEFAULT_STORE);
});

test("readChannelSectionsStore: object with wrong version returns DEFAULT_STORE", () => {
  const pubkey = "pk-wrong-version";
  window.localStorage.setItem(
    storageKey(pubkey),
    JSON.stringify({ version: 2, sections: [], assignments: {} }),
  );
  const result = readChannelSectionsStore(pubkey);
  assert.deepEqual(result, DEFAULT_STORE);
});

test("writeChannelSectionsStore: returns false when setItem throws", () => {
  const pubkey = "pk-throws";
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("storage full");
  };
  try {
    const result = writeChannelSectionsStore(pubkey, makeStore());
    assert.equal(result, false);
  } finally {
    window.localStorage.setItem = original;
  }
});

test("storageKey: returns expected format with pubkey", () => {
  assert.equal(storageKey("abc123"), "buzz-channel-sections.v1:abc123");
});

// ─── Relay-scoped key tests ───────────────────────────────────────────────────

test("storageKey: with relayUrl includes normalized+encoded relay in key", () => {
  const relay = "wss://relay.example.com";
  const key = storageKey("pk1", relay);
  assert.equal(
    key,
    `buzz-channel-sections.v1:pk1:${encodeURIComponent(normalizeRelayUrl(relay))}`,
  );
});

test("storageKey: without relayUrl returns legacy pubkey-only key", () => {
  assert.equal(storageKey("pk1"), "buzz-channel-sections.v1:pk1");
  assert.equal(storageKey("pk1", undefined), "buzz-channel-sections.v1:pk1");
});

test("storageKey: two different relays produce different keys for same pubkey", () => {
  const k1 = storageKey("pk1", "wss://relay-a.example.com");
  const k2 = storageKey("pk1", "wss://relay-b.example.com");
  assert.notEqual(k1, k2);
});

test("storageKey: equivalent relay URLs (case + trailing slash) map to the same key", () => {
  const k1 = storageKey("pk1", "WSS://Relay.Example/");
  const k2 = storageKey("pk1", "wss://relay.example");
  assert.equal(k1, k2);
});

test("readChannelSectionsStore + writeChannelSectionsStore: scoped write/read roundtrip", () => {
  const pubkey = "pk-relay-roundtrip";
  const relay = "wss://relay.example.com";
  const store = makeStore({
    sections: [makeSection({ id: "s1", name: "Work", order: 0 })],
    assignments: { chan1: "s1" },
  });
  assert.equal(writeChannelSectionsStore(pubkey, store, relay), true);
  const result = readChannelSectionsStore(pubkey, relay);
  assert.deepEqual(result, store);
});

test("readChannelSectionsStore: scoped key is isolated from other relay's data", () => {
  const pubkey = "pk-isolation";
  const relayA = "wss://relay-a.example.com";
  const relayB = "wss://relay-b.example.com";
  const storeA = makeStore({
    sections: [makeSection({ id: "sa", name: "Relay A section", order: 0 })],
    assignments: {},
  });
  writeChannelSectionsStore(pubkey, storeA, relayA);
  // Relay B should see empty store, not relay A's data.
  const resultB = readChannelSectionsStore(pubkey, relayB);
  assert.deepEqual(resultB, DEFAULT_STORE);
});

test("readChannelSectionsStore: migrates legacy unscoped data on first scoped read", () => {
  const pubkey = "pk-migrate";
  const relay = "wss://relay-migrate.example.com";
  const legacyStore = makeStore({
    sections: [makeSection({ id: "sl", name: "Legacy Section", order: 0 })],
    assignments: {},
  });
  // Write under legacy (pubkey-only) key.
  writeChannelSectionsStore(pubkey, legacyStore);
  // First scoped read should migrate and return the legacy data.
  const result = readChannelSectionsStore(pubkey, relay);
  assert.deepEqual(result, legacyStore);
  // Legacy key must be deleted after migration (globally one-time guarantee).
  const legacyKey = storageKey(pubkey);
  assert.equal(window.localStorage.getItem(legacyKey), null);
  // Subsequent scoped reads should hit the scoped key directly.
  const result2 = readChannelSectionsStore(pubkey, relay);
  assert.deepEqual(result2, legacyStore);
});

test("readChannelSectionsStore: migration is globally one-time — relay B sees DEFAULT_STORE after relay A migrates", () => {
  const pubkey = "pk-migrate-once";
  const relayA = "wss://relay-migrate-once-a.example.com";
  const relayB = "wss://relay-migrate-once-b.example.com";
  const legacyStore = makeStore({
    sections: [makeSection({ id: "sm", name: "Migrated Section", order: 0 })],
    assignments: {},
  });
  // Write under legacy key then migrate via relay A.
  writeChannelSectionsStore(pubkey, legacyStore);
  readChannelSectionsStore(pubkey, relayA); // triggers migration, deletes legacy key
  // Relay B must see DEFAULT_STORE — legacy data must not bleed in.
  const resultB = readChannelSectionsStore(pubkey, relayB);
  assert.deepEqual(resultB, DEFAULT_STORE);
  // Relay B scoped key must not have been created.
  const scopedBKey = storageKey(pubkey, relayB);
  assert.equal(window.localStorage.getItem(scopedBKey), null);
});

test("readChannelSectionsStore: migration only copies non-empty legacy stores", () => {
  const pubkey = "pk-migrate-empty";
  const relay = "wss://relay-migrate-empty.example.com";
  // Write an explicitly empty store under the legacy key.
  writeChannelSectionsStore(pubkey, DEFAULT_STORE);
  // Empty legacy store should NOT be migrated — we get DEFAULT_STORE.
  const result = readChannelSectionsStore(pubkey, relay);
  assert.deepEqual(result, DEFAULT_STORE);
});

test("readChannelSectionsStore: scoped key takes precedence over legacy key after migration", () => {
  const pubkey = "pk-precedence";
  const relay = "wss://relay-precedence.example.com";
  const legacyStore = makeStore({
    sections: [makeSection({ id: "sold", name: "Old Section", order: 0 })],
    assignments: {},
  });
  const newStore = makeStore({
    sections: [makeSection({ id: "snew", name: "New Section", order: 0 })],
    assignments: {},
  });
  // Write legacy data then scoped data (simulates post-migration state).
  writeChannelSectionsStore(pubkey, legacyStore);
  writeChannelSectionsStore(pubkey, newStore, relay);
  // Scoped read must return the scoped (newer) store, not the legacy one.
  const result = readChannelSectionsStore(pubkey, relay);
  assert.deepEqual(result, newStore);
});

test("parseChannelSectionPayload: preserves icon field when present", () => {
  const payload = {
    version: 1,
    sections: [{ id: "s1", name: "Work", icon: "🚀", order: 0 }],
    assignments: { chan1: "s1" },
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result, {
    version: 1,
    sections: [{ id: "s1", name: "Work", icon: "🚀", order: 0 }],
    assignments: { chan1: "s1" },
  });
});

test("parseChannelSectionPayload: omits icon field when empty or whitespace", () => {
  const payload = {
    version: 1,
    sections: [
      { id: "s1", name: "A", icon: "", order: 0 },
      { id: "s2", name: "B", icon: "   ", order: 1 },
      { id: "s3", name: "C", order: 2 },
    ],
    assignments: {},
  };
  const result = parseChannelSectionPayload(payload);
  assert.deepEqual(result?.sections, [
    { id: "s1", name: "A", order: 0 },
    { id: "s2", name: "B", order: 1 },
    { id: "s3", name: "C", order: 2 },
  ]);
});
