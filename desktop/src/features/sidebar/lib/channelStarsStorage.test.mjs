import assert from "node:assert/strict";
import test from "node:test";

import {
  parseStarPayload,
  mergeStores,
  starredChannelIdsFromStore,
} from "./channelStarsStorage.ts";

// ── parseStarPayload ──────────────────────────────────────────────────────────

test("parseStarPayload: valid payload with channels returns store", () => {
  const payload = {
    version: 1,
    channels: {
      "chan-1": { starred: true, updatedAt: 1000 },
      "chan-2": { starred: false, updatedAt: 2000 },
    },
  };
  const result = parseStarPayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      "chan-1": { starred: true, updatedAt: 1000 },
      "chan-2": { starred: false, updatedAt: 2000 },
    },
  });
});

test("parseStarPayload: missing version returns null", () => {
  assert.equal(
    parseStarPayload({
      channels: { "chan-1": { starred: true, updatedAt: 1 } },
    }),
    null,
  );
});

test("parseStarPayload: wrong version returns null", () => {
  assert.equal(
    parseStarPayload({
      version: 2,
      channels: { "chan-1": { starred: true, updatedAt: 1 } },
    }),
    null,
  );
});

test("parseStarPayload: null input returns null", () => {
  assert.equal(parseStarPayload(null), null);
});

test("parseStarPayload: non-object input returns null", () => {
  assert.equal(parseStarPayload("string"), null);
  assert.equal(parseStarPayload(42), null);
  assert.equal(parseStarPayload(true), null);
});

test("parseStarPayload: malformed channel entries missing starred/updatedAt are filtered out", () => {
  const payload = {
    version: 1,
    channels: {
      "no-starred": { updatedAt: 1000 },
      "no-updated-at": { starred: true },
      valid: { starred: false, updatedAt: 500 },
      "starred-wrong-type": { starred: "yes", updatedAt: 1000 },
      "updated-at-wrong-type": { starred: true, updatedAt: "now" },
      null: null,
    },
  };
  const result = parseStarPayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      valid: { starred: false, updatedAt: 500 },
    },
  });
});

test("parseStarPayload: NaN/Infinity/negative updatedAt entries are filtered out", () => {
  const payload = {
    version: 1,
    channels: {
      nan: { starred: true, updatedAt: NaN },
      inf: { starred: true, updatedAt: Infinity },
      "neg-inf": { starred: true, updatedAt: -Infinity },
      neg: { starred: true, updatedAt: -1 },
      valid: { starred: true, updatedAt: 100 },
    },
  };
  const result = parseStarPayload(payload);
  assert.deepEqual(result, {
    version: 1,
    channels: { valid: { starred: true, updatedAt: 100 } },
  });
});

test("parseStarPayload: empty channels returns store with empty channels", () => {
  const result = parseStarPayload({ version: 1, channels: {} });
  assert.deepEqual(result, { version: 1, channels: {} });
});

test("parseStarPayload: version 1 with no channels key returns store with empty channels", () => {
  const result = parseStarPayload({ version: 1 });
  assert.deepEqual(result, { version: 1, channels: {} });
});

// ── mergeStores ───────────────────────────────────────────────────────────────

test("mergeStores: non-overlapping channels returns union of both", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: true, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-b": { starred: false, updatedAt: 200 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result, {
    version: 1,
    channels: {
      "chan-a": { starred: true, updatedAt: 100 },
      "chan-b": { starred: false, updatedAt: 200 },
    },
  });
});

test("mergeStores: overlapping channel with remote newer takes remote", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: false, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { starred: true, updatedAt: 200 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], {
    starred: true,
    updatedAt: 200,
  });
});

test("mergeStores: overlapping channel with local newer takes local", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: true, updatedAt: 300 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { starred: false, updatedAt: 100 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], {
    starred: true,
    updatedAt: 300,
  });
});

test("mergeStores: overlapping channel with same updatedAt local wins", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: true, updatedAt: 500 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { starred: false, updatedAt: 500 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], {
    starred: true,
    updatedAt: 500,
  });
});

test("mergeStores: unstar with higher updatedAt overrides star", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: true, updatedAt: 100 } },
  };
  const remote = {
    version: 1,
    channels: { "chan-a": { starred: false, updatedAt: 999 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels["chan-a"], {
    starred: false,
    updatedAt: 999,
  });
});

test("mergeStores: empty local returns remote entries", () => {
  const local = { version: 1, channels: {} };
  const remote = {
    version: 1,
    channels: { "chan-b": { starred: true, updatedAt: 42 } },
  };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels, {
    "chan-b": { starred: true, updatedAt: 42 },
  });
});

test("mergeStores: empty remote returns local entries", () => {
  const local = {
    version: 1,
    channels: { "chan-a": { starred: false, updatedAt: 10 } },
  };
  const remote = { version: 1, channels: {} };
  const result = mergeStores(local, remote);
  assert.deepEqual(result.channels, {
    "chan-a": { starred: false, updatedAt: 10 },
  });
});

test("mergeStores: both empty returns empty", () => {
  const result = mergeStores(
    { version: 1, channels: {} },
    { version: 1, channels: {} },
  );
  assert.deepEqual(result, { version: 1, channels: {} });
});

// ── starredChannelIdsFromStore ────────────────────────────────────────────────

test("starredChannelIdsFromStore: returns set of IDs where starred=true", () => {
  const store = {
    version: 1,
    channels: {
      "chan-a": { starred: true, updatedAt: 100 },
      "chan-b": { starred: true, updatedAt: 200 },
      "chan-c": { starred: false, updatedAt: 300 },
    },
  };
  const result = starredChannelIdsFromStore(store);
  assert.equal(result.has("chan-a"), true);
  assert.equal(result.has("chan-b"), true);
  assert.equal(result.has("chan-c"), false);
  assert.equal(result.size, 2);
});

test("starredChannelIdsFromStore: excludes IDs where starred=false", () => {
  const store = {
    version: 1,
    channels: {
      "chan-x": { starred: false, updatedAt: 1 },
      "chan-y": { starred: false, updatedAt: 2 },
    },
  };
  const result = starredChannelIdsFromStore(store);
  assert.equal(result.size, 0);
});

test("starredChannelIdsFromStore: empty channels returns empty set", () => {
  const result = starredChannelIdsFromStore({ version: 1, channels: {} });
  assert.equal(result.size, 0);
});
