import assert from "node:assert/strict";
import test from "node:test";

import {
  bookmarkedIdsFromStore,
  mergeStores,
  parseBookmarkPayload,
  PREVIEW_MAX_LENGTH,
  savedEntriesFromStore,
} from "./bookmarksStorage.ts";

function entry(overrides = {}) {
  return {
    bookmarked: true,
    updatedAt: 100,
    channelId: "chan-1",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// parseBookmarkPayload — defensive parsing of untrusted (decrypted) JSON
// ---------------------------------------------------------------------------

test("parseBookmarkPayload: rejects non-objects and wrong version", () => {
  assert.equal(parseBookmarkPayload(null), null);
  assert.equal(parseBookmarkPayload(42), null);
  assert.equal(parseBookmarkPayload({ version: 2, bookmarks: {} }), null);
});

test("parseBookmarkPayload: keeps valid entries, drops malformed ones", () => {
  const store = parseBookmarkPayload({
    version: 1,
    bookmarks: {
      good: entry(),
      tombstone: entry({ bookmarked: false, updatedAt: 200 }),
      missingChannel: { bookmarked: true, updatedAt: 5 },
      badUpdatedAt: { bookmarked: true, updatedAt: -1, channelId: "c" },
      notBool: { bookmarked: "yes", updatedAt: 5, channelId: "c" },
    },
  });
  assert.deepEqual(Object.keys(store.bookmarks).sort(), ["good", "tombstone"]);
});

test("parseBookmarkPayload: missing/invalid bookmarks map yields empty store", () => {
  assert.deepEqual(parseBookmarkPayload({ version: 1 }), {
    version: 1,
    bookmarks: {},
  });
  assert.deepEqual(parseBookmarkPayload({ version: 1, bookmarks: [] }), {
    version: 1,
    bookmarks: {},
  });
});

test("parseBookmarkPayload: keeps threadRootId, drops empty string", () => {
  const store = parseBookmarkPayload({
    version: 1,
    bookmarks: {
      reply: entry({ threadRootId: "root-abc" }),
      empty: entry({ threadRootId: "" }),
    },
  });
  assert.equal(store.bookmarks.reply.threadRootId, "root-abc");
  assert.equal(store.bookmarks.empty.threadRootId, undefined);
});

test("parseBookmarkPayload: truncates oversized previews", () => {
  const long = "x".repeat(PREVIEW_MAX_LENGTH + 50);
  const store = parseBookmarkPayload({
    version: 1,
    bookmarks: { a: entry({ preview: long }) },
  });
  assert.equal(store.bookmarks.a.preview.length, PREVIEW_MAX_LENGTH);
});

// ---------------------------------------------------------------------------
// mergeStores — per-key last-write-wins by updatedAt
// ---------------------------------------------------------------------------

test("mergeStores: newer updatedAt wins per key; keys union", () => {
  const local = {
    version: 1,
    bookmarks: {
      shared: entry({ bookmarked: true, updatedAt: 100 }),
      localOnly: entry({ updatedAt: 10 }),
    },
  };
  const remote = {
    version: 1,
    bookmarks: {
      shared: entry({ bookmarked: false, updatedAt: 200 }),
      remoteOnly: entry({ updatedAt: 20 }),
    },
  };
  const merged = mergeStores(local, remote);
  // Remote's newer tombstone wins for the shared key.
  assert.equal(merged.bookmarks.shared.bookmarked, false);
  assert.equal(merged.bookmarks.shared.updatedAt, 200);
  assert.ok(merged.bookmarks.localOnly);
  assert.ok(merged.bookmarks.remoteOnly);
});

test("mergeStores: equal updatedAt keeps local (>=)", () => {
  const local = { version: 1, bookmarks: { a: entry({ channelId: "L" }) } };
  const remote = { version: 1, bookmarks: { a: entry({ channelId: "R" }) } };
  assert.equal(mergeStores(local, remote).bookmarks.a.channelId, "L");
});

// ---------------------------------------------------------------------------
// selectors — tombstones excluded, savedEntries newest-first
// ---------------------------------------------------------------------------

test("bookmarkedIdsFromStore: excludes tombstones", () => {
  const store = {
    version: 1,
    bookmarks: {
      on: entry({ bookmarked: true }),
      off: entry({ bookmarked: false }),
    },
  };
  const ids = bookmarkedIdsFromStore(store);
  assert.ok(ids.has("on"));
  assert.ok(!ids.has("off"));
});

test("savedEntriesFromStore: newest-first, tombstones excluded", () => {
  const store = {
    version: 1,
    bookmarks: {
      older: entry({ updatedAt: 100 }),
      newer: entry({ updatedAt: 300 }),
      middle: entry({ updatedAt: 200 }),
      removed: entry({ bookmarked: false, updatedAt: 999 }),
    },
  };
  const order = savedEntriesFromStore(store).map((e) => e.eventId);
  assert.deepEqual(order, ["newer", "middle", "older"]);
});
