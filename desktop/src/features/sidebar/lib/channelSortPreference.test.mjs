import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_SORT_MODE,
  DEFAULT_STORE,
  parseChannelSortPayload,
  sectionSortGroupKey,
  sortChannelsForSidebar,
  sortModeForGroup,
  stripOrphanedSectionModes,
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

test("parseChannelSortPayload: non-object input returns null", () => {
  assert.equal(parseChannelSortPayload(null), null);
  assert.equal(parseChannelSortPayload("alpha"), null);
  assert.equal(parseChannelSortPayload(42), null);
});

// ── sortModeForGroup / defaults ──────────────────────────────────────────────

test("default sort mode is alpha and default store has no overrides", () => {
  assert.equal(DEFAULT_SORT_MODE, "alpha");
  assert.deepEqual(DEFAULT_STORE.groups, {});
});

test("sortModeForGroup: unset group falls back to alpha", () => {
  assert.equal(sortModeForGroup(DEFAULT_STORE, "channels"), "alpha");
  assert.equal(sortModeForGroup(DEFAULT_STORE, "dms"), "alpha");
});

test("sortModeForGroup: set groups are independent", () => {
  const store = {
    version: 1,
    groups: { channels: "recent", [sectionSortGroupKey("abc")]: "recent" },
  };
  assert.equal(sortModeForGroup(store, "channels"), "recent");
  assert.equal(sortModeForGroup(store, sectionSortGroupKey("abc")), "recent");
  assert.equal(sortModeForGroup(store, "forums"), "alpha");
  assert.equal(sortModeForGroup(store, sectionSortGroupKey("xyz")), "alpha");
});

test("sectionSortGroupKey: namespaced by section id", () => {
  assert.equal(sectionSortGroupKey("abc"), "section:abc");
});

// ── stripOrphanedSectionModes ────────────────────────────────────────────────

test("stripOrphanedSectionModes: drops modes for deleted sections", () => {
  const store = {
    version: 1,
    groups: {
      channels: "recent",
      [sectionSortGroupKey("live")]: "recent",
      [sectionSortGroupKey("deleted")]: "alpha",
    },
  };
  assert.deepEqual(stripOrphanedSectionModes(store, ["live"]), {
    version: 1,
    groups: { channels: "recent", [sectionSortGroupKey("live")]: "recent" },
  });
});

test("stripOrphanedSectionModes: fixed groups survive with no live sections", () => {
  const store = {
    version: 1,
    groups: {
      starred: "recent",
      channels: "alpha",
      forums: "recent",
      dms: "recent",
      [sectionSortGroupKey("gone")]: "recent",
    },
  };
  assert.deepEqual(stripOrphanedSectionModes(store, []), {
    version: 1,
    groups: {
      starred: "recent",
      channels: "alpha",
      forums: "recent",
      dms: "recent",
    },
  });
});

test("stripOrphanedSectionModes: returns same reference when nothing is stale", () => {
  const store = {
    version: 1,
    groups: { channels: "recent", [sectionSortGroupKey("live")]: "recent" },
  };
  assert.equal(stripOrphanedSectionModes(store, ["live", "other"]), store);
});

test("stripOrphanedSectionModes: does not mutate the input store", () => {
  const store = {
    version: 1,
    groups: { [sectionSortGroupKey("gone")]: "recent" },
  };
  stripOrphanedSectionModes(store, []);
  assert.deepEqual(store.groups, { [sectionSortGroupKey("gone")]: "recent" });
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
