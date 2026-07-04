import assert from "node:assert/strict";
import test from "node:test";

import {
  channelSnapshotKey,
  readChannelSnapshot,
  removeChannelSnapshotForRelay,
  writeChannelSnapshot,
} from "./channelSnapshot.ts";

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

test("channelSnapshotKey: normalizes trailing slash and case", () => {
  assert.equal(
    channelSnapshotKey("WSS://Relay.Example.com/"),
    channelSnapshotKey("wss://relay.example.com"),
  );
});

test("read after write returns the persisted channels", () => {
  const channels = [makeChannel(), makeChannel({ id: "chan-2", name: "Dev" })];
  writeChannelSnapshot(RELAY, channels);
  assert.deepEqual(readChannelSnapshot(RELAY), channels);
});

test("read for an unknown relay returns null", () => {
  assert.equal(readChannelSnapshot("wss://never-written.example.com"), null);
});

test("read returns null for malformed JSON", () => {
  window.localStorage.setItem(channelSnapshotKey(RELAY), "not-json{{{");
  assert.equal(readChannelSnapshot(RELAY), null);
});

test("read returns null for a wrong-version payload", () => {
  window.localStorage.setItem(
    channelSnapshotKey(RELAY),
    JSON.stringify({ version: 2, channels: [makeChannel()] }),
  );
  assert.equal(readChannelSnapshot(RELAY), null);
});

test("read returns null when channels is not an array", () => {
  window.localStorage.setItem(
    channelSnapshotKey(RELAY),
    JSON.stringify({ version: 1, channels: "nope" }),
  );
  assert.equal(readChannelSnapshot(RELAY), null);
});

test("remove clears the snapshot for that relay", () => {
  writeChannelSnapshot(RELAY, [makeChannel()]);
  removeChannelSnapshotForRelay(RELAY);
  assert.equal(readChannelSnapshot(RELAY), null);
});

test("write is tolerant of storage failures", () => {
  const original = window.localStorage.setItem;
  window.localStorage.setItem = () => {
    throw new Error("quota exceeded");
  };
  try {
    assert.doesNotThrow(() => writeChannelSnapshot(RELAY, [makeChannel()]));
  } finally {
    window.localStorage.setItem = original;
  }
});
