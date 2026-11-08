import assert from "node:assert/strict";
import test from "node:test";

import {
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

test("parseChannelSortPayload: valid per-group payload", () => {
  assert.deepEqual(
    parseChannelSortPayload({
      version: 1,
      groups: { channels: "recent", dms: "alpha" },
    }),
    { version: 1, groups: { channels: "recent", dms: "alpha" } },
  );
});

test("parseChannelSortPayload: empty groups is valid", () => {
  assert.deepEqual(parseChannelSortPayload({ version: 1, groups: {} }), {
    version: 1,
    groups: {},
  });
});

test("parseChannelSortPayload: unknown modes are filtered out", () => {
  assert.deepEqual(
    parseChannelSortPayload({
      version: 1,
      groups: { channels: "zorp", forums: "recent", dms: 42 },
    }),
    { version: 1, groups: { forums: "recent" } },
  );
});

test("parseChannelSortPayload: missing/invalid groups falls back to empty", () => {
  assert.deepEqual(parseChannelSortPayload({ version: 1 }), {
    version: 1,
    groups: {},
  });
  assert.deepEqual(parseChannelSortPayload({ version: 1, groups: ["x"] }), {
    version: 1,
    groups: {},
  });
});

test("parseChannelSortPayload: wrong version returns null", () => {
  assert.equal(
    parseChannelSortPayload({ version: 2, groups: { channels: "alpha" } }),
    null,
  );
});

test("alpha: sorts case-insensitively with deterministic code-unit collation", () => {
  const sorted = sortChannelsForSidebar(
    [
      makeChannel("2", "zeta"),
      makeChannel("3", "Alpha"),
      makeChannel("1", "alpha"),
      makeChannel("4", "Éclair"),
    ],
    "alpha",
  );
  assert.deepEqual(
    sorted.map((c) => c.id),
    ["1", "3", "2", "4"],
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
