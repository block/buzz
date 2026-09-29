import assert from "node:assert/strict";
import test from "node:test";

import {
  channelIdsClaimedByOtherSections,
  ensureExclusiveChannelMembership,
  isCommunitySectionPickerChannel,
  reorderCommunitySections,
} from "./sectionAdmin.ts";

test("isCommunitySectionPickerChannel keeps ordinary streams", () => {
  assert.equal(
    isCommunitySectionPickerChannel({
      id: "c1",
      name: "general",
      channelType: "stream",
      ttlSeconds: null,
    }),
    true,
  );
});

test("isCommunitySectionPickerChannel excludes huddle backing streams", () => {
  assert.equal(
    isCommunitySectionPickerChannel({
      id: "h1",
      name: "general huddle",
      channelType: "stream",
      ttlSeconds: 3600,
    }),
    false,
  );
  assert.equal(
    isCommunitySectionPickerChannel({
      id: "h2",
      name: "huddle-abcdef12",
      channelType: "stream",
      ttlSeconds: 60,
    }),
    false,
  );
});

test("isCommunitySectionPickerChannel excludes forums", () => {
  assert.equal(
    isCommunitySectionPickerChannel({
      id: "f1",
      name: "forum",
      channelType: "forum",
      ttlSeconds: null,
    }),
    false,
  );
});

test("channelIdsClaimedByOtherSections skips the editing section", () => {
  const claimed = channelIdsClaimedByOtherSections(
    [
      { id: "a", name: "A", order: 0, channelIds: ["c1", "c2"] },
      { id: "b", name: "B", order: 1, channelIds: ["c3"] },
    ],
    "a",
  );
  assert.deepEqual([...claimed].sort(), ["c3"]);
});

test("ensureExclusiveChannelMembership keeps first section wins", () => {
  const next = ensureExclusiveChannelMembership([
    { id: "a", name: "A", order: 0, channelIds: ["c1", "c2"] },
    { id: "b", name: "B", order: 1, channelIds: ["c2", "c3"] },
  ]);
  assert.deepEqual(next[0].channelIds, ["c1", "c2"]);
  assert.deepEqual(next[1].channelIds, ["c3"]);
});

test("reorderCommunitySections rewrites order indexes", () => {
  const next = reorderCommunitySections(
    [
      { id: "a", name: "A", order: 0, channelIds: [] },
      { id: "b", name: "B", order: 1, channelIds: [] },
      { id: "c", name: "C", order: 2, channelIds: [] },
    ],
    ["c", "a", "b"],
  );
  assert.deepEqual(
    next.map((s) => ({ id: s.id, order: s.order })),
    [
      { id: "c", order: 0 },
      { id: "a", order: 1 },
      { id: "b", order: 2 },
    ],
  );
});
