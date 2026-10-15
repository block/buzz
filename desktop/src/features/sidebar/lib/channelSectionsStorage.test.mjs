import assert from "node:assert/strict";
import test from "node:test";

import {
  parseChannelSectionPayload,
  storageKey,
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
