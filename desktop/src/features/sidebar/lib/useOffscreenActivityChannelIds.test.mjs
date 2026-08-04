import assert from "node:assert/strict";
import test from "node:test";

import { getOffscreenActivityChannelIds } from "./useOffscreenActivityChannelIds.ts";
import { getSidebarActivityOverflowLabel } from "./useSidebarActivityOverflow.ts";

test("keeps every unread channel navigable while adding working activity", () => {
  const activity = getOffscreenActivityChannelIds({
    activeWorkingByChannelId: new Map([["working", {}]]),
    previewActivityChannelIds: new Set(["preview"]),
    unreadChannelIds: new Set(["dm", "forum", "stream"]),
  });

  assert.deepEqual([...activity.messageChannelIds].sort(), [
    "dm",
    "forum",
    "preview",
    "stream",
  ]);
  assert.deepEqual([...activity.channelIds].sort(), [
    "dm",
    "forum",
    "preview",
    "stream",
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
