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
  assert.equal(storageKey("abc123"), "sprout-channel-sections.v1:abc123");
});
