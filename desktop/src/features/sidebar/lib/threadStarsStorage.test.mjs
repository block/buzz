import assert from "node:assert/strict";
import test from "node:test";

import {
  boundThreadStarStore,
  MAX_THREAD_STAR_ENTRIES,
  parseThreadStarPayload,
  starredThreadEntriesFromStore,
  starredThreadIdsFromStore,
} from "./threadStarsStorage.ts";

test("parseThreadStarPayload: valid payload returns store", () => {
  const payload = {
    version: 1,
    threads: {
      "root-1": {
        rootId: "root-1",
        channelId: "chan-1",
        title: "Hello thread",
        channelName: "general",
        starredAt: 1000,
      },
    },
  };
  assert.deepEqual(parseThreadStarPayload(payload), {
    version: 1,
    threads: payload.threads,
  });
});

test("parseThreadStarPayload: wrong version returns null", () => {
  assert.equal(
    parseThreadStarPayload({
      version: 2,
      threads: {},
    }),
    null,
  );
});

test("parseThreadStarPayload: key/rootId mismatch is filtered", () => {
  const result = parseThreadStarPayload({
    version: 1,
    threads: {
      "wrong-key": {
        rootId: "root-1",
        channelId: "chan-1",
        title: "Hello",
        channelName: "general",
        starredAt: 1,
      },
      "root-2": {
        rootId: "root-2",
        channelId: "chan-2",
        title: "Kept",
        channelName: "ops",
        starredAt: 2,
      },
    },
  });
  assert.deepEqual(result, {
    version: 1,
    threads: {
      "root-2": {
        rootId: "root-2",
        channelId: "chan-2",
        title: "Kept",
        channelName: "ops",
        starredAt: 2,
      },
    },
  });
});

test("parseThreadStarPayload: malformed entries are filtered", () => {
  const result = parseThreadStarPayload({
    version: 1,
    threads: {
      "no-channel": {
        rootId: "no-channel",
        title: "x",
        channelName: "g",
        starredAt: 1,
      },
      valid: {
        rootId: "valid",
        channelId: "c",
        title: "ok",
        channelName: "g",
        starredAt: 5,
      },
    },
  });
  assert.deepEqual(Object.keys(result.threads), ["valid"]);
});

test("boundThreadStarStore: preserves newest entries and optional key", () => {
  const threads = {};
  for (let i = 0; i < MAX_THREAD_STAR_ENTRIES + 5; i += 1) {
    const rootId = `root-${i}`;
    threads[rootId] = {
      rootId,
      channelId: "c",
      title: `t${i}`,
      channelName: "g",
      starredAt: i,
    };
  }
  const bounded = boundThreadStarStore({ version: 1, threads }, "root-0");
  assert.equal(Object.keys(bounded.threads).length, MAX_THREAD_STAR_ENTRIES);
  assert.ok(bounded.threads["root-0"]);
  assert.ok(bounded.threads[`root-${MAX_THREAD_STAR_ENTRIES + 4}`]);
  assert.equal(bounded.threads["root-1"], undefined);
});

test("starredThreadEntriesFromStore: newest first", () => {
  const entries = starredThreadEntriesFromStore({
    version: 1,
    threads: {
      a: {
        rootId: "a",
        channelId: "c",
        title: "A",
        channelName: "g",
        starredAt: 10,
      },
      b: {
        rootId: "b",
        channelId: "c",
        title: "B",
        channelName: "g",
        starredAt: 30,
      },
      c: {
        rootId: "c",
        channelId: "c",
        title: "C",
        channelName: "g",
        starredAt: 20,
      },
    },
  });
  assert.deepEqual(
    entries.map((entry) => entry.rootId),
    ["b", "c", "a"],
  );
});

test("starredThreadIdsFromStore: returns root ids", () => {
  const ids = starredThreadIdsFromStore({
    version: 1,
    threads: {
      r1: {
        rootId: "r1",
        channelId: "c",
        title: "t",
        channelName: "g",
        starredAt: 1,
      },
    },
  });
  assert.deepEqual([...ids], ["r1"]);
});
