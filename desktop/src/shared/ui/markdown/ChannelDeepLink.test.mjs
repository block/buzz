import assert from "node:assert/strict";
import test from "node:test";

import { channelTooltipFooter } from "./ChannelDeepLink.tsx";

const channel = {
  id: "channel-id",
  name: "history",
  channelType: "forum",
  visibility: "private",
  description: "",
  topic: null,
  purpose: null,
  memberCount: 0,
  memberPubkeys: [],
  lastMessageAt: null,
  archivedAt: "2026-08-17T00:00:00Z",
  participants: [],
  participantPubkeys: [],
  isMember: true,
  ttlSeconds: null,
  ttlDeadline: null,
};

test("channelTooltipFooter adds archived status without changing existing metadata", () => {
  assert.equal(
    channelTooltipFooter(channel),
    "Private channel · Forum · Archived",
  );
  assert.equal(
    channelTooltipFooter({ ...channel, archivedAt: null }),
    "Private channel · Forum",
  );
});

test("channelTooltipFooter uses just now for activity within a minute", () => {
  assert.equal(
    channelTooltipFooter({
      ...channel,
      archivedAt: null,
      lastMessageAt: new Date().toISOString(),
    }),
    "Private channel · Forum · Active just now",
  );
});
