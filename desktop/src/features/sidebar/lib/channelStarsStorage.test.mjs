// Compact stars lane adapter contract.
// Full merge-lane storage invariants are in mergeLaneStorage.shared.test.mjs.
// This file proves only stars-specific wiring: value field name, storage key prefix, and id projection.

import assert from "node:assert/strict";
import test from "node:test";

import {
  parseStarPayload,
  starredChannelIdsFromStore,
  storageKey,
} from "./channelStarsStorage.ts";

test("stars adapter: value field is 'starred', key prefix is buzz-channel-stars.v1", () => {
  const raw = {
    version: 1,
    channels: { a: { starred: true, updatedAt: 100, rev: 1 } },
  };
  const parsed = parseStarPayload(raw);
  assert.ok(parsed !== null, "valid stars payload parses");
  assert.equal(parsed.channels.a.starred, true, "starred field preserved");
  assert.equal(
    storageKey("pk1"),
    "buzz-channel-stars.v1:pk1",
    "storage key prefix",
  );
});

test("stars adapter: idsFromStore projects starred=true entries", () => {
  const store = {
    version: 1,
    channels: {
      a: { starred: true, updatedAt: 1, rev: 0 },
      b: { starred: false, updatedAt: 2, rev: 0 },
    },
  };
  const ids = starredChannelIdsFromStore(store);
  assert.ok(ids.has("a"), "starred channel in set");
  assert.ok(!ids.has("b"), "unstarred channel excluded");
});

test("stars adapter: wrong value field (muted) is rejected by parser", () => {
  const mutePayload = {
    version: 1,
    channels: { a: { muted: true, updatedAt: 100, rev: 1 } },
  };
  const result = parseStarPayload(mutePayload);
  assert.deepEqual(
    result?.channels ?? {},
    {},
    "muted entry filtered as malformed",
  );
});
