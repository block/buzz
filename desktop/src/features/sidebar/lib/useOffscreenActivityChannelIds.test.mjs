import assert from "node:assert/strict";
import test from "node:test";

import { getOffscreenActivityChannelIds } from "./useOffscreenActivityChannelIds.ts";
import { getSidebarActivityOverflowLabel } from "./useSidebarActivityOverflow.ts";

function makeChannel(id, channelType = "stream") {
  return { channelType, id };
}

test("keeps working channels separate from message activity", () => {
  const activity = getOffscreenActivityChannelIds({
    activeWorkingByChannelId: new Map([["working", {}]]),
    channels: [makeChannel("dm", "dm")],
    previewActivityChannelIds: new Set(["preview"]),
    unreadChannelIds: new Set(["dm"]),
  });

  assert.deepEqual([...activity.messageChannelIds].sort(), ["dm", "preview"]);
  assert.deepEqual([...activity.channelIds].sort(), [
    "dm",
    "preview",
    "working",
  ]);
});

test("uses an activity-neutral overflow label when work contributes", () => {
  assert.equal(
    getSidebarActivityOverflowLabel({ activityCount: 2, messageCount: 1 }),
    "2 new activity",
  );
  assert.equal(
    getSidebarActivityOverflowLabel({ activityCount: 1, messageCount: 1 }),
    undefined,
  );
});
