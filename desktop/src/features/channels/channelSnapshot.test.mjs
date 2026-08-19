import assert from "node:assert/strict";
import test from "node:test";

import {
  channelSnapshotKey,
  inspectChannelSnapshot,
  readChannelSnapshot,
  removeChannelSnapshotForRelay,
  writeChannelSnapshot,
} from "./channelSnapshot.ts";

if (typeof globalThis.window === "undefined") {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      get length() {
        return storage.size;
      },
      key: (index) => [...storage.keys()][index] ?? null,
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    },
  };
}

function makeChannel(overrides = {}) {
  return {
    id: "chan-1",
    name: "General",
    channelType: "stream",
    visibility: "public",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 3,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

const RELAY = "wss://relay.example.com";
const OWNER = "ABCDEF";
const HASH = "channels-hash-v2";

test.beforeEach(() => {
  removeChannelSnapshotForRelay(RELAY);
  removeChannelSnapshotForRelay("wss://other.example.com");
});

test("channelSnapshotKey: normalizes trailing slash and case", () => {
  assert.equal(
    channelSnapshotKey("WSS://Relay.Example.com/", OWNER),
    channelSnapshotKey("wss://relay.example.com", OWNER.toLowerCase()),
  );
});

test("read after write returns the complete list and matching hash", () => {
  const channels = Array.from({ length: 14 }, (_, index) =>
    makeChannel({ id: `chan-${index}`, name: `Channel ${index}` }),
  );

  writeChannelSnapshot(RELAY, OWNER, channels, HASH);

  const snapshot = readChannelSnapshot(RELAY, OWNER.toLowerCase());
  assert.deepEqual(snapshot, { channels, hash: HASH });
  assert.equal(snapshot.channels.length, channels.length);

  const { diagnostics } = inspectChannelSnapshot(RELAY, OWNER);
  assert.equal(diagnostics.presence, "present");
  assert.equal(diagnostics.channelCount, channels.length);
  assert.ok(diagnostics.serializedBytes > 0);
  assert.ok(diagnostics.ageMs >= 0);
});

test("inspection distinguishes absent from invalid snapshots", () => {
  assert.deepEqual(inspectChannelSnapshot(RELAY, OWNER), {
    diagnostics: {
      ageMs: 0,
      channelCount: 0,
      presence: "absent",
      serializedBytes: 0,
    },
    snapshot: null,
  });

  window.localStorage.setItem(channelSnapshotKey(RELAY, OWNER), "not-json{{{");
  const invalid = inspectChannelSnapshot(RELAY, OWNER);
  assert.equal(invalid.snapshot, null);
  assert.equal(invalid.diagnostics.presence, "invalid");
  assert.ok(invalid.diagnostics.serializedBytes > 0);
});

test("inspection memoizes parsing for repeated reads of the same document", () => {
  writeChannelSnapshot(RELAY, OWNER, [makeChannel()], "memo-hash");
  const originalParse = JSON.parse;
  let parses = 0;
  JSON.parse = (...args) => {
    parses += 1;
    return originalParse(...args);
  };
  try {
    assert.notEqual(inspectChannelSnapshot(RELAY, OWNER).snapshot, null);
    assert.notEqual(inspectChannelSnapshot(RELAY, OWNER).snapshot, null);
    assert.equal(parses, 1);
  } finally {
    JSON.parse = originalParse;
  }
});

test("inspection tolerates denied storage without retrying the read", () => {
  const original = window.localStorage;
  let reads = 0;
  window.localStorage = {
    get length() {
      return 0;
    },
    key: () => null,
    getItem: () => {
      reads += 1;
      throw new DOMException("storage denied", "SecurityError");
    },
    setItem: () => {},
    removeItem: () => {},
  };
  try {
    assert.deepEqual(inspectChannelSnapshot(RELAY, OWNER), {
      diagnostics: {
        ageMs: 0,
        channelCount: 0,
        presence: "absent",
        serializedBytes: 0,
      },
      snapshot: null,
    });
    assert.equal(reads, 1);
  } finally {
    window.localStorage = original;
  }
});

test("write replaces list and hash in lockstep", () => {
  writeChannelSnapshot(RELAY, OWNER, [makeChannel()], "old-hash");
  const channels = [
    makeChannel({ id: "chan-2", name: "Dev" }),
    makeChannel({ id: "chan-3", name: "Design" }),
  ];

  writeChannelSnapshot(RELAY, OWNER, channels, "new-hash");

  assert.deepEqual(readChannelSnapshot(RELAY, OWNER), {
    channels,
    hash: "new-hash",
  });
});

test("read for an unknown relay returns null", () => {
  assert.equal(
    readChannelSnapshot("wss://never-written.example.com", OWNER),
    null,
  );
});

test("read rejects malformed JSON", () => {
  window.localStorage.setItem(channelSnapshotKey(RELAY, OWNER), "not-json{{{");
  assert.equal(readChannelSnapshot(RELAY, OWNER), null);
});

test("read rejects a legacy channel-only snapshot", () => {
  window.localStorage.setItem(
    channelSnapshotKey(RELAY, OWNER),
    JSON.stringify({ version: 1, channels: [makeChannel()] }),
  );
  assert.equal(readChannelSnapshot(RELAY, OWNER), null);
});

test("read rejects a snapshot whose hash and list were partially changed", () => {
  writeChannelSnapshot(RELAY, OWNER, [makeChannel()], "old-hash");
  const key = channelSnapshotKey(RELAY, OWNER);
  const stored = JSON.parse(window.localStorage.getItem(key));
  window.localStorage.setItem(
    key,
    JSON.stringify({ ...stored, hash: "newer-partial-hash" }),
  );

  assert.equal(readChannelSnapshot(RELAY, OWNER), null);
  assert.equal(
    inspectChannelSnapshot(RELAY, OWNER).diagnostics.presence,
    "invalid",
  );
});

test("read rejects a v2 snapshot with a missing hash", () => {
  window.localStorage.setItem(
    channelSnapshotKey(RELAY, OWNER),
    JSON.stringify({
      version: 2,
      updatedAt: Date.now(),
      ownerPubkey: OWNER,
      channels: [makeChannel()],
    }),
  );
  assert.equal(readChannelSnapshot(RELAY, OWNER), null);
});

test("read rejects a snapshot owned by another identity", () => {
  writeChannelSnapshot(RELAY, OWNER, [makeChannel()], HASH);
  assert.equal(readChannelSnapshot(RELAY, "different-owner"), null);
});

test("same-relay identities retain independent snapshots", () => {
  const ownerAChannels = [makeChannel({ id: "owner-a", name: "Owner A" })];
  const ownerBChannels = [makeChannel({ id: "owner-b", name: "Owner B" })];

  writeChannelSnapshot(RELAY, OWNER, ownerAChannels, "owner-a-hash");
  writeChannelSnapshot(
    RELAY,
    "different-owner",
    ownerBChannels,
    "owner-b-hash",
  );

  assert.deepEqual(readChannelSnapshot(RELAY, OWNER), {
    channels: ownerAChannels,
    hash: "owner-a-hash",
  });
  assert.deepEqual(readChannelSnapshot(RELAY, "different-owner"), {
    channels: ownerBChannels,
    hash: "owner-b-hash",
  });
});

test("remove clears every identity snapshot for that relay", () => {
  writeChannelSnapshot(RELAY, OWNER, [makeChannel()], HASH);
  writeChannelSnapshot(
    RELAY,
    "different-owner",
    [makeChannel({ id: "owner-b" })],
    "owner-b-hash",
  );
  writeChannelSnapshot(
    "wss://other.example.com",
    OWNER,
    [makeChannel({ id: "other-relay" })],
    "other-relay-hash",
  );
  removeChannelSnapshotForRelay(RELAY);
  assert.equal(readChannelSnapshot(RELAY, OWNER), null);
  assert.equal(readChannelSnapshot(RELAY, "different-owner"), null);
  assert.notEqual(readChannelSnapshot("wss://other.example.com", OWNER), null);
});

test("cache write evicts disposable entries and retries at quota", () => {
  const original = window.localStorage;
  const storage = new Map([
    ["buzz-channel-messages.v1:relay:old", "big"],
    ["buzz-timeline-skeleton-shape.v1:old", "small"],
  ]);
  window.localStorage = {
    get length() {
      return storage.size;
    },
    key: (index) => [...storage.keys()][index] ?? null,
    getItem: (key) => storage.get(key) ?? null,
    setItem(key, value) {
      if (!storage.has(key) && storage.size >= 2) {
        throw new Error("quota exceeded");
      }
      storage.set(key, value);
    },
    removeItem: (key) => storage.delete(key),
  };
  try {
    writeChannelSnapshot(RELAY, OWNER, [makeChannel()], HASH);
    assert.deepEqual(readChannelSnapshot(RELAY, OWNER), {
      channels: [makeChannel()],
      hash: HASH,
    });
    assert.equal(storage.has("buzz-channel-messages.v1:relay:old"), false);
    assert.equal(storage.has("buzz-timeline-skeleton-shape.v1:old"), false);
  } finally {
    window.localStorage = original;
  }
});

test("write is tolerant of storage failures", () => {
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("quota exceeded");
  };
  try {
    assert.doesNotThrow(() =>
      writeChannelSnapshot(RELAY, OWNER, [makeChannel()], HASH),
    );
  } finally {
    window.localStorage.setItem = original;
  }
});
