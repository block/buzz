import assert from "node:assert/strict";
import test from "node:test";

import { getOffscreenActivityChannelIds } from "./useOffscreenActivityChannelIds.ts";
import {
  getOffscreenMentionCount,
  getSidebarActivityOverflowLabel,
  hasHighPriorityOverflow,
} from "./useSidebarActivityOverflow.ts";

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

test("counts offscreen mentions without treating unread DMs as mentions", () => {
  const highPriority = new Set(["dm", "mention", "mention-without-count"]);
  const dmChannels = new Set(["dm"]);
  const counts = new Map([
    ["dm", 3],
    ["mention", 2],
    ["channel", 8],
  ]);

  assert.equal(
    getOffscreenMentionCount(
      ["dm", "mention", "mention-without-count", "channel"],
      dmChannels,
      highPriority,
      counts,
    ),
    2,
  );
});

test("promotes only when the offscreen set includes actionable unread", () => {
  const actionable = new Set(["dm", "mention"]);

  assert.equal(
    hasHighPriorityOverflow(["channel", "working"], actionable),
    false,
  );
  assert.equal(hasHighPriorityOverflow(["channel", "dm"], actionable), true);
  assert.equal(hasHighPriorityOverflow(["mention"], actionable), true);
});
