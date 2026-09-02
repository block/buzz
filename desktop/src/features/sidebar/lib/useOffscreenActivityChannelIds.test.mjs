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

test("keeps working-only channels out of message overflow prioritization", () => {
  const activity = getOffscreenActivityChannelIds({
    activeWorkingByChannelId: new Map([["read-working-dm", {}]]),
    previewActivityChannelIds: new Set(),
    unreadChannelIds: new Set(["unread-channel"]),
  });

  assert.deepEqual([...activity.messageChannelIds], ["unread-channel"]);
  assert.deepEqual(
    [...activity.channelIds],
    ["unread-channel", "read-working-dm"],
  );
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
