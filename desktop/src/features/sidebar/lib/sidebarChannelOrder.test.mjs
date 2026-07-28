import assert from "node:assert/strict";
import test from "node:test";

import {
  adjacentSidebarChannelId,
  buildSidebarChannelGroups,
  flattenSidebarChannelGroups,
} from "./sidebarChannelOrder.ts";

function makeChannel(id, name, lastMessageAt = null) {
  return { id, name, channelType: "stream", lastMessageAt };
}

function makeSection(id, name, order) {
  return { id, name, order };
}

const defaultSort = () => "alpha";

function orderedIds({
  streamChannels,
  starredChannelIds = undefined,
  sections = [],
  assignments = {},
  sortModeFor = defaultSort,
}) {
  const groups = buildSidebarChannelGroups({
    streamChannels,
    starredChannelIds,
    sections,
    assignments,
    sortModeFor,
  });
  return flattenSidebarChannelGroups(groups, sections).map((c) => c.id);
}

test("flattens starred, then sections in order, then unassigned", () => {
  const streamChannels = [
    makeChannel("uz", "zeta"),
    makeChannel("s1", "starred-one"),
    makeChannel("b1", "bravo"),
    makeChannel("a1", "alpha"),
    makeChannel("ua", "apple"),
  ];
  const sections = [makeSection("sec-a", "A", 0), makeSection("sec-b", "B", 1)];
  const ids = orderedIds({
    streamChannels,
    starredChannelIds: new Set(["s1"]),
    sections,
    assignments: { a1: "sec-a", b1: "sec-b" },
  });
  assert.deepEqual(ids, ["s1", "a1", "b1", "ua", "uz"]);
});

test("starred channels are excluded from their assigned section", () => {
  const streamChannels = [
    makeChannel("a1", "alpha"),
    makeChannel("b1", "bravo"),
  ];
  const sections = [makeSection("sec-a", "A", 0)];
  const groups = buildSidebarChannelGroups({
    streamChannels,
    starredChannelIds: new Set(["a1"]),
    sections,
    assignments: { a1: "sec-a", b1: "sec-a" },
    sortModeFor: defaultSort,
  });
  assert.deepEqual(
    groups.starred.map((c) => c.id),
    ["a1"],
  );
  assert.deepEqual(
    groups.bySection["sec-a"].map((c) => c.id),
    ["b1"],
  );
});

test("channels assigned to a deleted section fall back to unassigned", () => {
  const ids = orderedIds({
    streamChannels: [makeChannel("a1", "alpha"), makeChannel("b1", "bravo")],
    sections: [],
    assignments: { a1: "gone-section" },
  });
  assert.deepEqual(ids, ["a1", "b1"]);
});

test("each grouping honors its own sort preference", () => {
  const streamChannels = [
    makeChannel("old", "old", "2024-01-01T00:00:00Z"),
    makeChannel("new", "new", "2025-01-01T00:00:00Z"),
    makeChannel("a1", "alpha"),
    makeChannel("z1", "zulu"),
  ];
  const sections = [makeSection("sec-a", "A", 0)];
  const ids = orderedIds({
    streamChannels,
    sections,
    assignments: { old: "sec-a", new: "sec-a" },
    // Section sorts by recency (newest first), unassigned stays alphabetical.
    sortModeFor: (group) => (group === "section:sec-a" ? "recent" : "alpha"),
  });
  assert.deepEqual(ids, ["new", "old", "a1", "z1"]);
});

test("sections flatten in the provided display order", () => {
  const streamChannels = [
    makeChannel("a1", "alpha"),
    makeChannel("b1", "bravo"),
  ];
  const sections = [makeSection("sec-b", "B", 0), makeSection("sec-a", "A", 1)];
  const ids = orderedIds({
    streamChannels,
    sections,
    assignments: { a1: "sec-a", b1: "sec-b" },
  });
  assert.deepEqual(ids, ["b1", "a1"]);
});

test("adjacent: steps down and up through the ordered list", () => {
  const channels = [
    makeChannel("a", "a"),
    makeChannel("b", "b"),
    makeChannel("c", "c"),
  ];
  assert.equal(adjacentSidebarChannelId(channels, "a", 1), "b");
  assert.equal(adjacentSidebarChannelId(channels, "b", 1), "c");
  assert.equal(adjacentSidebarChannelId(channels, "c", -1), "b");
});

test("adjacent: skips muted channels in both directions", () => {
  const channels = [
    makeChannel("a", "a"),
    makeChannel("b", "b"),
    makeChannel("c", "c"),
    makeChannel("d", "d"),
  ];
  const muted = new Set(["b", "c"]);
  assert.equal(adjacentSidebarChannelId(channels, "a", 1, muted), "d");
  assert.equal(adjacentSidebarChannelId(channels, "d", -1, muted), "a");
});

test("adjacent: stops at both ends without wrapping", () => {
  const channels = [makeChannel("a", "a"), makeChannel("b", "b")];
  assert.equal(adjacentSidebarChannelId(channels, "a", -1), null);
  assert.equal(adjacentSidebarChannelId(channels, "b", 1), null);
});

test("adjacent: returns null when only muted channels remain toward the end", () => {
  const channels = [makeChannel("a", "a"), makeChannel("b", "b")];
  assert.equal(
    adjacentSidebarChannelId(channels, "a", 1, new Set(["b"])),
    null,
  );
});

test("adjacent: no-op when no channel is selected or it is not in the list", () => {
  const channels = [makeChannel("a", "a")];
  assert.equal(adjacentSidebarChannelId(channels, null, 1), null);
  assert.equal(adjacentSidebarChannelId(channels, "missing", 1), null);
  assert.equal(adjacentSidebarChannelId([], "a", 1), null);
});

test("adjacent: navigates from a muted active channel", () => {
  const channels = [makeChannel("a", "a"), makeChannel("b", "b")];
  assert.equal(adjacentSidebarChannelId(channels, "a", 1, new Set(["a"])), "b");
});
