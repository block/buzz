import assert from "node:assert/strict";
import test from "node:test";

import {
  channelsForCommunitySection,
  communitySectionChannelIds,
} from "./sidebarBuckets.ts";

test("communitySectionChannelIds unions all listed channels", () => {
  const ids = communitySectionChannelIds([
    { id: "a", name: "A", order: 0, channelIds: ["c1", "c2"] },
    { id: "b", name: "B", order: 1, channelIds: ["c2", "c3"] },
  ]);
  assert.deepEqual([...ids].sort(), ["c1", "c2", "c3"]);
});

test("channelsForCommunitySection skips starred and missing", () => {
  const map = new Map([
    ["c1", { id: "c1", name: "One" }],
    ["c2", { id: "c2", name: "Two" }],
  ]);
  const channels = channelsForCommunitySection(
    { id: "a", name: "A", order: 0, channelIds: ["c1", "c2", "missing"] },
    map,
    new Set(["c1"]),
  );
  assert.deepEqual(
    channels.map((c) => c.id),
    ["c2"],
  );
});
