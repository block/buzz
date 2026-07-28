import assert from "node:assert/strict";
import test from "node:test";

import {
  aggregateUnreadChannels,
  makeObservedUnreadEvent,
} from "./unreadChannelCounts.ts";

const CHANNEL = "channel-1";

function observed(overrides = {}) {
  const event = makeObservedUnreadEvent({
    id: overrides.id ?? "event-1",
    createdAt: overrides.createdAt ?? 100,
    rootId: overrides.rootId ?? null,
    highPriority: overrides.highPriority ?? false,
    directMention: overrides.directMention ?? false,
    channelType: overrides.channelType,
    isThreadedReply: overrides.isThreadedReply ?? false,
  });
  return [event.id, event];
}

function aggregate({
  channels = [{ id: CHANNEL, channelType: "stream" }],
  events = [observed()],
  forced = [],
  muted = [],
  activeChannelId = null,
} = {}) {
  const byChannel = new Map([[CHANNEL, new Map(events)]]);
  return aggregateUnreadChannels({
    channels,
    activeChannelId,
    hasForcedUnread: (id) => forced.includes(id),
    hasObservedLatest: (id) => byChannel.has(id),
    getObservedEvents: (id) => byChannel.get(id),
    getReadAt: () => () => null,
    isMutedChannel: (id) => muted.includes(id),
  });
}

test("an unmuted channel contributes its dot, badge, and app-badge counts", () => {
  const result = aggregate({ events: [observed({ highPriority: true })] });
  assert.deepEqual([...result.unreadChannelIds], [CHANNEL]);
  assert.deepEqual([...result.highPriorityUnreadChannelIds], [CHANNEL]);
  assert.equal(result.unreadChannelCounts.get(CHANNEL), 1);
  assert.equal(result.unreadChannelNotificationCount, 1);
});

test("a muted channel contributes nothing for ordinary posts", () => {
  const result = aggregate({ muted: [CHANNEL] });
  assert.equal(result.unreadChannelIds.size, 0);
  assert.equal(result.unreadChannelCounts.size, 0);
  assert.equal(result.unreadChannelNotificationCount, 0);
});

test("a muted channel still contributes its mention-tier events", () => {
  const result = aggregate({
    muted: [CHANNEL],
    events: [
      observed({ id: "plain" }),
      observed({ id: "mention", highPriority: true, directMention: true }),
    ],
  });
  assert.deepEqual([...result.unreadChannelIds], [CHANNEL]);
  assert.deepEqual([...result.highPriorityUnreadChannelIds], [CHANNEL]);
  // Only the mention counts — the ordinary post stays suppressed.
  assert.equal(result.unreadChannelCounts.get(CHANNEL), 1);
  assert.equal(result.unreadChannelNotificationCount, 1);
});

test("a muted channel drops a frozen broadcast-reply highPriority row", () => {
  // Observed at level "all" (so the ladder froze highPriority:true), then the
  // user mutes the channel. A broadcast reply / @channel marker is not
  // mention-tier below level "all", so it must retire — only direct p-tag
  // mentions pierce a mute.
  const result = aggregate({
    muted: [CHANNEL],
    events: [
      observed({ id: "broadcast-reply", highPriority: true, rootId: "root-1" }),
    ],
  });
  assert.equal(result.unreadChannelIds.size, 0);
  assert.equal(result.highPriorityUnreadChannelIds.size, 0);
  assert.equal(result.unreadChannelCounts.size, 0);
  assert.equal(result.unreadChannelNotificationCount, 0);
});

test("a muted channel counts only the direct mention beside a frozen broadcast row", () => {
  const result = aggregate({
    muted: [CHANNEL],
    events: [
      observed({ id: "broadcast-reply", highPriority: true }),
      observed({ id: "mention", highPriority: true, directMention: true }),
    ],
  });
  assert.deepEqual([...result.unreadChannelIds], [CHANNEL]);
  assert.deepEqual([...result.highPriorityUnreadChannelIds], [CHANNEL]);
  assert.equal(result.unreadChannelCounts.get(CHANNEL), 1);
  assert.equal(result.unreadChannelNotificationCount, 1);
});

test("a muted channel drops its forced-unread dot", () => {
  const result = aggregate({ muted: [CHANNEL], forced: [CHANNEL] });
  assert.equal(result.unreadChannelIds.size, 0);
});

test("mute never applies to a DM channel", () => {
  const result = aggregate({
    channels: [{ id: CHANNEL, channelType: "dm" }],
    muted: [CHANNEL],
    events: [observed({ channelType: "dm" })],
  });
  assert.deepEqual([...result.unreadChannelIds], [CHANNEL]);
  assert.deepEqual([...result.highPriorityUnreadChannelIds], [CHANNEL]);
});

test("the active channel is always excluded", () => {
  const result = aggregate({ activeChannelId: CHANNEL });
  assert.equal(result.unreadChannelIds.size, 0);
});
