import assert from "node:assert/strict";
import test from "node:test";

import {
  boundChannelSectionsStore,
  DEFAULT_STORE,
  MAX_CHANNEL_SECTION_ASSIGNMENTS,
  MAX_CHANNEL_SECTIONS,
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

test("parseChannelSectionPayload: valid, invalid versions, assignment filtering, orphaned stripping, icon handling", () => {
  const payload = {
    version: 1,
    sections: [{ id: "s1", name: "Work", order: 0 }],
    assignments: { chan1: "s1" },
  };
  assert.deepEqual(parseChannelSectionPayload(payload), payload);
  // Carl P1.2: a future schema version must not be accepted as v1 state.
  for (const input of [
    null,
    "string",
    42,
    { version: 2, sections: [], assignments: {} },
    { sections: [], assignments: {} },
    { version: 0, sections: [], assignments: {} },
  ])
    assert.equal(parseChannelSectionPayload(input), null);
  assert.deepEqual(
    parseChannelSectionPayload({
      version: 1,
      sections: [makeSection()],
      assignments: { chan1: "s1", chan2: 42, chan3: null, chan4: true },
    })?.assignments,
    { chan1: "s1" },
  );
  assert.deepEqual(
    parseChannelSectionPayload({
      version: 1,
      sections: [{ id: "s1", name: "Exists", order: 0 }],
      assignments: { chan1: "s1", chan2: "missing-section" },
    })?.assignments,
    { chan1: "s1" },
  );
  assert.deepEqual(
    parseChannelSectionPayload({
      version: 1,
      sections: [{ id: "s1", name: "Work", icon: "🚀", order: 0 }],
      assignments: { chan1: "s1" },
    }),
    {
      version: 1,
      sections: [{ id: "s1", name: "Work", icon: "🚀", order: 0 }],
      assignments: { chan1: "s1" },
    },
  );
  assert.deepEqual(
    parseChannelSectionPayload({
      version: 1,
      sections: [
        { id: "s1", name: "A", icon: "", order: 0 },
        { id: "s2", name: "B", icon: "   ", order: 1 },
      ],
      assignments: {},
    })?.sections,
    [
      { id: "s1", name: "A", order: 0 },
      { id: "s2", name: "B", order: 1 },
    ],
  );
});

test("stripOrphanedAssignments: returns same reference when no orphans; new object when orphans present", () => {
  for (const store of [
    makeStore({
      sections: [makeSection({ id: "s1" })],
      assignments: { chan1: "s1" },
    }),
    makeStore({
      sections: [
        makeSection({ id: "s1" }),
        makeSection({ id: "s2", name: "B", order: 1 }),
      ],
      assignments: { chan1: "s1", chan2: "s2" },
    }),
    makeStore({ sections: [], assignments: {} }),
  ])
    assert.equal(stripOrphanedAssignments(store), store);
  const store = makeStore({
    sections: [makeSection({ id: "s1" })],
    assignments: { chan1: "s1", chan2: "ghost" },
  });
  const result = stripOrphanedAssignments(store);
  assert.notEqual(result, store);
  assert.deepEqual(result.assignments, { chan1: "s1" });
});

test("boundChannelSectionsStore caps sections and assignments", () => {
  const sections = Array.from({ length: MAX_CHANNEL_SECTIONS + 1 }, (_, i) =>
    makeSection({ id: `section-${i}`, order: i }),
  );
  const assignments = Object.fromEntries(
    Array.from({ length: MAX_CHANNEL_SECTION_ASSIGNMENTS + 1 }, (_, i) => [
      `channel-${i}`,
      "section-100",
    ]),
  );
  const bounded = boundChannelSectionsStore(
    makeStore({ sections, assignments }),
  );
  assert.equal(bounded.sections.length, MAX_CHANNEL_SECTIONS);
  assert.equal(
    bounded.sections.some((s) => s.id === "section-0"),
    false,
  );
  assert.equal(
    Object.keys(bounded.assignments).length,
    MAX_CHANNEL_SECTION_ASSIGNMENTS,
  );
  assert.equal(bounded.assignments["channel-0"], undefined);
});

test("write + read: legacy (no relay) roundtrip; returns false when setItem throws", () => {
  const store = makeStore({
    sections: [makeSection({ id: "s1", name: "Work", order: 0 })],
    assignments: { chan1: "s1" },
  });
  assert.equal(writeChannelSectionsStore("pk-roundtrip", store), true);
  assert.deepEqual(readChannelSectionsStore("pk-roundtrip"), store);
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("storage full");
  };
  try {
    assert.equal(writeChannelSectionsStore("pk-throws", makeStore()), false);
  } finally {
    window.localStorage.setItem = original;
  }
});

test("readChannelSectionsStore: non-existent key, corrupt JSON, wrong version all return DEFAULT_STORE", () => {
  assert.deepEqual(
    readChannelSectionsStore("pk-does-not-exist-xyz"),
    DEFAULT_STORE,
  );
  window.localStorage.setItem(storageKey("pk-corrupt"), "not-valid-json{{{");
  assert.deepEqual(readChannelSectionsStore("pk-corrupt"), DEFAULT_STORE);
  window.localStorage.setItem(
    storageKey("pk-wrong-version"),
    JSON.stringify({ version: 2, sections: [], assignments: {} }),
  );
  assert.deepEqual(readChannelSectionsStore("pk-wrong-version"), DEFAULT_STORE);
});

test("scoped roundtrip, isolation, migration, and precedence", () => {
  assert.equal(storageKey("abc123"), "buzz-channel-sections.v1:abc123");
  assert.equal(
    storageKey("pk1", "wss://relay.example.com"),
    `buzz-channel-sections.v1:pk1:${encodeURIComponent(normalizeRelayUrl("wss://relay.example.com"))}`,
  );
  assert.notEqual(
    storageKey("pk1", "wss://relay-a.example.com"),
    storageKey("pk1", "wss://relay-b.example.com"),
  );
  assert.equal(
    storageKey("pk1", "WSS://Relay.Example/"),
    storageKey("pk1", "wss://relay.example"),
  );
  const store = makeStore({
    sections: [makeSection({ id: "s1", name: "Work", order: 0 })],
    assignments: { chan1: "s1" },
  });
  assert.equal(
    writeChannelSectionsStore("pk-rr", store, "wss://relay.example.com"),
    true,
  );
  assert.deepEqual(
    readChannelSectionsStore("pk-rr", "wss://relay.example.com"),
    store,
  );
  writeChannelSectionsStore(
    "pk-iso",
    makeStore({
      sections: [makeSection({ id: "sa", name: "A", order: 0 })],
      assignments: {},
    }),
    "wss://relay-a.example.com",
  );
  assert.deepEqual(
    readChannelSectionsStore("pk-iso", "wss://relay-b.example.com"),
    DEFAULT_STORE,
  );
  const legacyStore = makeStore({
    sections: [makeSection({ id: "sl", name: "Legacy", order: 0 })],
    assignments: {},
  });
  writeChannelSectionsStore("pk-mig", legacyStore);
  assert.deepEqual(
    readChannelSectionsStore("pk-mig", "wss://relay-mig.example.com"),
    legacyStore,
  );
  assert.equal(
    window.localStorage.getItem(storageKey("pk-mig")),
    null,
    "legacy key deleted",
  );
  assert.deepEqual(
    readChannelSectionsStore("pk-mig", "wss://relay-mig.example.com"),
    legacyStore,
    "idempotent",
  );
  writeChannelSectionsStore(
    "pk-once",
    makeStore({
      sections: [makeSection({ id: "sm", name: "M", order: 0 })],
      assignments: {},
    }),
  );
  readChannelSectionsStore("pk-once", "wss://relay-once-a.example.com");
  assert.deepEqual(
    readChannelSectionsStore("pk-once", "wss://relay-once-b.example.com"),
    DEFAULT_STORE,
    "one-time",
  );
  writeChannelSectionsStore("pk-empty", DEFAULT_STORE);
  assert.deepEqual(
    readChannelSectionsStore("pk-empty", "wss://relay-empty.example.com"),
    DEFAULT_STORE,
  );
  const relay = "wss://relay-prec.example.com";
  writeChannelSectionsStore(
    "pk-prec",
    makeStore({ sections: [makeSection({ id: "sold" })], assignments: {} }),
  );
  const newStore = makeStore({
    sections: [makeSection({ id: "snew", name: "New", order: 0 })],
    assignments: {},
  });
  writeChannelSectionsStore("pk-prec", newStore, relay);
  assert.deepEqual(readChannelSectionsStore("pk-prec", relay), newStore);
});
