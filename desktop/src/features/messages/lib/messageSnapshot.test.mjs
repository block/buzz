import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeHistoryOverSnapshot,
  messageSnapshotKey,
  readMessageSnapshot,
  removeMessageSnapshotsForRelay,
  writeMessageSnapshot,
} from "./messageSnapshot.ts";

if (typeof globalThis.window === "undefined") {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
      key: (index) => [...storage.keys()][index] ?? null,
      get length() {
        return storage.size;
      },
    },
  };
}

function makeEvent(overrides = {}) {
  return {
    id: `event-${Math.random().toString(36).slice(2)}`,
    pubkey: "pubkey-1",
    created_at: 1_700_000_000,
    kind: 9,
    tags: [["h", "chan-1"]],
    content: "hello",
    sig: "sig",
    ...overrides,
  };
}

const RELAY = "wss://relay.example.com";

function clearRelay(relayUrl = RELAY) {
  removeMessageSnapshotsForRelay(relayUrl);
}

test("messageSnapshotKey: normalizes trailing slash and case", () => {
  assert.equal(
    messageSnapshotKey("WSS://Relay.Example.com/", "chan-1"),
    messageSnapshotKey("wss://relay.example.com", "chan-1"),
  );
});

test("read after write returns the persisted events", () => {
  clearRelay();
  const events = [makeEvent({ id: "a" }), makeEvent({ id: "b" })];
  writeMessageSnapshot(RELAY, "chan-1", events);
  assert.deepEqual(readMessageSnapshot(RELAY, "chan-1"), events);
});

test("read for an unknown channel returns null", () => {
  clearRelay();
  assert.equal(readMessageSnapshot(RELAY, "chan-never"), null);
});

test("read returns null for malformed JSON", () => {
  window.localStorage.setItem(
    messageSnapshotKey(RELAY, "chan-bad"),
    "not-json{{{",
  );
  assert.equal(readMessageSnapshot(RELAY, "chan-bad"), null);
});

test("read returns null for a wrong-version payload", () => {
  window.localStorage.setItem(
    messageSnapshotKey(RELAY, "chan-v2"),
    JSON.stringify({ version: 2, updatedAt: 1, events: [makeEvent()] }),
  );
  assert.equal(readMessageSnapshot(RELAY, "chan-v2"), null);
});

test("pending optimistic events are not persisted", () => {
  clearRelay();
  const settled = makeEvent({ id: "settled" });
  writeMessageSnapshot(RELAY, "chan-1", [
    settled,
    makeEvent({ id: "optimistic", pending: true }),
  ]);
  assert.deepEqual(readMessageSnapshot(RELAY, "chan-1"), [settled]);
});

test("write with only pending events persists nothing", () => {
  clearRelay();
  writeMessageSnapshot(RELAY, "chan-1", [makeEvent({ pending: true })]);
  assert.equal(readMessageSnapshot(RELAY, "chan-1"), null);
});

test("snapshot keeps only the newest slice of a long timeline", () => {
  clearRelay();
  const events = Array.from({ length: 200 }, (_, i) =>
    makeEvent({ id: `event-${i}`, created_at: 1_700_000_000 + i }),
  );
  writeMessageSnapshot(RELAY, "chan-1", events);
  const persisted = readMessageSnapshot(RELAY, "chan-1");
  assert.equal(persisted.length, 80);
  assert.equal(persisted[persisted.length - 1].id, "event-199");
  assert.equal(persisted[0].id, "event-120");
});

test("per-relay channel cap evicts the least recently written snapshot", () => {
  clearRelay();
  for (let i = 0; i < 21; i++) {
    writeMessageSnapshot(RELAY, `chan-${i}`, [makeEvent({ id: `e-${i}` })]);
  }
  // chan-0 was written first (oldest updatedAt tie broken by insertion) —
  // with 21 channels, at least one of the earliest must be evicted and the
  // newest retained.
  assert.notEqual(readMessageSnapshot(RELAY, "chan-20"), null);
  const retained = Array.from({ length: 21 }, (_, i) =>
    readMessageSnapshot(RELAY, `chan-${i}`),
  ).filter((snapshot) => snapshot !== null);
  assert.equal(retained.length, 20);
});

test("remove clears every snapshot for that relay only", () => {
  clearRelay();
  clearRelay("wss://other.example.com");
  writeMessageSnapshot(RELAY, "chan-1", [makeEvent({ id: "keep-other" })]);
  writeMessageSnapshot("wss://other.example.com", "chan-1", [
    makeEvent({ id: "other" }),
  ]);
  removeMessageSnapshotsForRelay(RELAY);
  assert.equal(readMessageSnapshot(RELAY, "chan-1"), null);
  assert.notEqual(
    readMessageSnapshot("wss://other.example.com", "chan-1"),
    null,
  );
});

test("write is tolerant of storage failures", () => {
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("quota exceeded");
  };
  try {
    assert.doesNotThrow(() =>
      writeMessageSnapshot(RELAY, "chan-1", [makeEvent()]),
    );
  } finally {
    window.localStorage.setItem = original;
  }
});

test("cold snapshot load: merge keeps snapshot-only rows and widens aux backfill to them", () => {
  const snapshotOnly = makeEvent({ id: "ghost", created_at: 1_700_000_000 });
  const fresh = makeEvent({ id: "fresh", created_at: 1_700_000_100 });
  const { merged, auxBackfillWindow } = mergeHistoryOverSnapshot({
    cached: undefined,
    snapshot: [snapshotOnly],
    history: [fresh],
  });
  assert.deepEqual(
    merged.map((event) => event.id),
    ["ghost", "fresh"],
  );
  assert.ok(auxBackfillWindow.some((event) => event.id === "ghost"));
  assert.ok(auxBackfillWindow.some((event) => event.id === "fresh"));
});

test("warm load: aux backfill stays scoped to the fresh window", () => {
  const cached = makeEvent({ id: "cached", created_at: 1_700_000_000 });
  const fresh = makeEvent({ id: "fresh", created_at: 1_700_000_100 });
  const { merged, auxBackfillWindow } = mergeHistoryOverSnapshot({
    cached: [cached],
    snapshot: [makeEvent({ id: "stale-snapshot" })],
    history: [fresh],
  });
  assert.ok(merged.some((event) => event.id === "cached"));
  assert.deepEqual(
    auxBackfillWindow.map((event) => event.id),
    ["fresh"],
  );
});

test("cold load without a snapshot backfills the fresh window only", () => {
  const fresh = makeEvent({ id: "fresh" });
  const { merged, auxBackfillWindow } = mergeHistoryOverSnapshot({
    cached: undefined,
    snapshot: null,
    history: [fresh],
  });
  assert.deepEqual(
    merged.map((event) => event.id),
    ["fresh"],
  );
  assert.deepEqual(
    auxBackfillWindow.map((event) => event.id),
    ["fresh"],
  );
});
