import assert from "node:assert/strict";
import test from "node:test";

import {
  boundChannelSortStore,
  DEFAULT_SORT_MODE,
  DEFAULT_STORE,
  MAX_CHANNEL_SORT_GROUPS,
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

test("parseChannelSortPayload: valid, empty, unknown modes filtered, invalid groups, wrong version, non-object", () => {
  assert.deepEqual(
    parseChannelSortPayload({
      version: 1,
      groups: { channels: "recent", dms: "alpha" },
    }),
    { version: 1, groups: { channels: "recent", dms: "alpha" } },
  );
  assert.deepEqual(parseChannelSortPayload({ version: 1, groups: {} }), {
    version: 1,
    groups: {},
  });
  assert.deepEqual(
    parseChannelSortPayload({
      version: 1,
      groups: { channels: "zorp", forums: "recent", dms: 42 },
    }),
    { version: 1, groups: { forums: "recent" } },
    "unknown/non-string modes filtered",
  );
  assert.deepEqual(parseChannelSortPayload({ version: 1 }), {
    version: 1,
    groups: {},
  });
  assert.deepEqual(parseChannelSortPayload({ version: 1, groups: ["x"] }), {
    version: 1,
    groups: {},
  });
  assert.equal(
    parseChannelSortPayload({ version: 2, groups: { channels: "alpha" } }),
    null,
  );
  assert.equal(parseChannelSortPayload(null), null);
  assert.equal(parseChannelSortPayload("alpha"), null);
  assert.equal(parseChannelSortPayload(42), null);
});

// ── sortModeForGroup / defaults / stripOrphanedSectionModes ─────────────────

test("defaults, sortModeForGroup, and stripOrphanedSectionModes", () => {
  assert.equal(DEFAULT_SORT_MODE, "alpha");
  assert.deepEqual(DEFAULT_STORE.groups, {});
  assert.equal(sortModeForGroup(DEFAULT_STORE, "channels"), "alpha");
  assert.equal(sectionSortGroupKey("abc"), "section:abc");
  const store = {
    version: 1,
    groups: { channels: "recent", [sectionSortGroupKey("abc")]: "recent" },
  };
  assert.equal(sortModeForGroup(store, "channels"), "recent");
  assert.equal(sortModeForGroup(store, "forums"), "alpha");
  assert.equal(sortModeForGroup(store, sectionSortGroupKey("xyz")), "alpha");
  // stripOrphanedSectionModes: drops deleted sections, preserves fixed groups.
  assert.deepEqual(
    stripOrphanedSectionModes(
      {
        version: 1,
        groups: {
          channels: "recent",
          [sectionSortGroupKey("live")]: "recent",
          [sectionSortGroupKey("deleted")]: "alpha",
        },
      },
      ["live"],
    ),
    {
      version: 1,
      groups: { channels: "recent", [sectionSortGroupKey("live")]: "recent" },
    },
  );
  assert.deepEqual(
    stripOrphanedSectionModes(
      {
        version: 1,
        groups: {
          starred: "recent",
          channels: "alpha",
          forums: "recent",
          dms: "recent",
          [sectionSortGroupKey("gone")]: "recent",
        },
      },
      [],
    ),
    {
      version: 1,
      groups: {
        starred: "recent",
        channels: "alpha",
        forums: "recent",
        dms: "recent",
      },
    },
  );
  const noOrphans = {
    version: 1,
    groups: { channels: "recent", [sectionSortGroupKey("live")]: "recent" },
  };
  assert.equal(
    stripOrphanedSectionModes(noOrphans, ["live", "other"]),
    noOrphans,
  );
});

test("boundChannelSortStore caps custom sections while preserving fixed groups", () => {
  const groups = {
    channels: "recent",
    ...Object.fromEntries(
      Array.from({ length: MAX_CHANNEL_SORT_GROUPS }, (_, index) => [
        sectionSortGroupKey(String(index)),
        "alpha",
      ]),
    ),
  };
  const bounded = boundChannelSortStore({ version: 1, groups });
  assert.equal(Object.keys(bounded.groups).length, MAX_CHANNEL_SORT_GROUPS);
  assert.equal(bounded.groups.channels, "recent");
  assert.equal(bounded.groups[sectionSortGroupKey("0")], undefined);
  assert.equal(bounded.groups[sectionSortGroupKey("99")], "alpha");
});

// ── sortChannelsForSidebar ───────────────────────────────────────────────────

for (const { title, channels, mode, expectedIds } of [
  {
    title: "alpha: case-insensitive deterministic code-unit collation",
    mode: "alpha",
    channels: [
      makeChannel("2", "zeta"),
      makeChannel("3", "Alpha"),
      makeChannel("1", "alpha"),
      makeChannel("4", "Éclair"),
    ],
    expectedIds: ["1", "3", "2", "4"],
  },
  {
    title: "recent: newest last message first",
    mode: "recent",
    channels: [
      makeChannel("old", "old", "2026-01-01T00:00:00Z"),
      makeChannel("new", "new", "2026-06-01T00:00:00Z"),
      makeChannel("mid", "mid", "2026-03-01T00:00:00Z"),
    ],
    expectedIds: ["new", "mid", "old"],
  },
  {
    title: "recent: channels without activity sink to bottom alphabetically",
    mode: "recent",
    channels: [
      makeChannel("quiet-z", "zzz"),
      makeChannel("active", "active", "2026-06-01T00:00:00Z"),
      makeChannel("quiet-a", "aaa"),
    ],
    expectedIds: ["active", "quiet-a", "quiet-z"],
  },
  {
    title: "recent: equal timestamps fall back to name then id",
    mode: "recent",
    channels: [
      makeChannel("b", "same", "2026-06-01T00:00:00Z"),
      makeChannel("a", "same", "2026-06-01T00:00:00Z"),
      makeChannel("c", "aardvark", "2026-06-01T00:00:00Z"),
    ],
    expectedIds: ["c", "a", "b"],
  },
  {
    title: "recent: unparseable timestamps treated as no activity",
    mode: "recent",
    channels: [
      makeChannel("bad", "bad", "not-a-date"),
      makeChannel("good", "good", "2026-06-01T00:00:00Z"),
    ],
    expectedIds: ["good", "bad"],
  },
]) {
  test(`sortChannelsForSidebar: ${title}`, () =>
    assert.deepEqual(
      sortChannelsForSidebar(channels, mode).map((c) => c.id),
      expectedIds,
    ));
}

test("sortChannelsForSidebar: does not mutate the input array", () => {
  const input = [makeChannel("b", "bbb"), makeChannel("a", "aaa")];
  sortChannelsForSidebar(input, "alpha");
  assert.deepEqual(
    input.map((c) => c.id),
    ["b", "a"],
  );
});
