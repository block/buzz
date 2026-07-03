import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_STORE,
  parseChannelSortPayload,
  sortChannelsForSidebar,
} from "./channelSortPreference.ts";

function makeChannel(id, name, lastMessageAt = null) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    id,
    isMember: true,
    lastMessageAt,
    memberCount: 2,
    memberPubkeys: [],
    name,
    participantPubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "open",
  };
}

// ── parseChannelSortPayload ──────────────────────────────────────────────────

test("parseChannelSortPayload: valid alpha payload", () => {
  assert.deepEqual(parseChannelSortPayload({ version: 1, mode: "alpha" }), {
    version: 1,
    mode: "alpha",
  });
});

test("parseChannelSortPayload: valid recent payload", () => {
  assert.deepEqual(parseChannelSortPayload({ version: 1, mode: "recent" }), {
    version: 1,
    mode: "recent",
  });
});

test("parseChannelSortPayload: unknown mode returns null", () => {
  assert.equal(parseChannelSortPayload({ version: 1, mode: "zorp" }), null);
});

test("parseChannelSortPayload: wrong version returns null", () => {
  assert.equal(parseChannelSortPayload({ version: 2, mode: "alpha" }), null);
});

test("parseChannelSortPayload: non-object input returns null", () => {
  assert.equal(parseChannelSortPayload(null), null);
  assert.equal(parseChannelSortPayload("alpha"), null);
  assert.equal(parseChannelSortPayload(42), null);
});

test("default store mode is alpha", () => {
  assert.equal(DEFAULT_STORE.mode, "alpha");
});

// ── sortChannelsForSidebar ───────────────────────────────────────────────────

test("alpha: sorts by name with id tie-breaker", () => {
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("2", "zeta"),
      makeChannel("1", "alpha"),
      makeChannel("b", "same"),
      makeChannel("a", "same"),
    ],
    "alpha",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["1", "a", "b", "2"],
  );
});

test("recent: newest last message first", () => {
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("old", "old", "2026-01-01T00:00:00Z"),
      makeChannel("new", "new", "2026-06-01T00:00:00Z"),
      makeChannel("mid", "mid", "2026-03-01T00:00:00Z"),
    ],
    "recent",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["new", "mid", "old"],
  );
});

test("recent: channels without activity sink to bottom alphabetically", () => {
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("quiet-z", "zzz"),
      makeChannel("active", "active", "2026-06-01T00:00:00Z"),
      makeChannel("quiet-a", "aaa"),
    ],
    "recent",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["active", "quiet-a", "quiet-z"],
  );
});

test("recent: equal timestamps fall back to name then id", () => {
  const ts = "2026-06-01T00:00:00Z";
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("b", "same", ts),
      makeChannel("a", "same", ts),
      makeChannel("c", "aardvark", ts),
    ],
    "recent",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["c", "a", "b"],
  );
});

test("recent: unparseable timestamps are treated as no activity", () => {
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("bad", "bad", "not-a-date"),
      makeChannel("good", "good", "2026-06-01T00:00:00Z"),
    ],
    "recent",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["good", "bad"],
  );
});

test("does not mutate the input array", () => {
  const input = [makeChannel("b", "bbb"), makeChannel("a", "aaa")];
  sortChannelsForSidebar(input, "alpha");
  assert.deepEqual(
    input.map((c) => c.id),
    ["b", "a"],
  );
});
