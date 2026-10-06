import assert from "node:assert/strict";
import test from "node:test";

import { sanitizeNotificationSettings } from "./hooks.ts";
import {
  buildHomeBadgeFeedItems,
  isHomeBadgeFeedItemUnread,
  resolveHomeBadgeFeedItemReadAt,
  shouldCountTowardHomeBadgeSubtotal,
} from "./lib/homeBadge.ts";

const ROOT_TAGS = [
  ["h", "stream-channel"],
  ["e", "root-event", "", "root"],
  ["e", "parent-event", "", "reply"],
];

const feedItem = (id, category = "activity") => ({
  id,
  kind: 9,
  pubkey: "author",
  content: id,
  createdAt: 1,
  channelId: null,
  channelName: "",
  tags: [],
  category,
});

const homeFeed = (feed) => ({
  feed: {
    mentions: [],
    needsAction: [],
    activity: [],
    agentActivity: [],
    ...feed,
  },
  meta: { since: 0, total: 0, generatedAt: 0 },
});

test("home badge excludes thread activity already shown in a channel preview", () => {
  const items = buildHomeBadgeFeedItems(
    homeFeed({
      mentions: [feedItem("mention", "mention")],
      needsAction: [feedItem("needs-action", "needs_action")],
      activity: [
        feedItem("locally-unread-activity"),
        feedItem("read-activity"),
      ],
      agentActivity: [
        feedItem("locally-unread-agent", "agent_activity"),
        feedItem("read-agent", "agent_activity"),
      ],
    }),
    [
      {
        ...feedItem("thread-activity"),
        tags: ROOT_TAGS,
      },
    ],
    new Set(["locally-unread-activity", "locally-unread-agent"]),
  );

  assert.deepEqual(
    items.map((item) => item.id),
    [
      "mention",
      "needs-action",
      "locally-unread-activity",
      "locally-unread-agent",
    ],
  );
});

test("home badge subtotal excludes channel-counted high-priority items", () => {
  const highPriorityChannelIds = new Set(["dm-channel", "stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
    ),
    false,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "dm-channel", channelType: "dm", tags: ROOT_TAGS },
      highPriorityChannelIds,
    ),
    false,
  );
});

test("home badge subtotal still counts non-DM thread-only rows", () => {
  const highPriorityChannelIds = new Set(["stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: ROOT_TAGS },
      highPriorityChannelIds,
    ),
    true,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "main-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
    ),
    true,
  );
  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: null, channelType: undefined, tags: [] },
      highPriorityChannelIds,
    ),
    true,
  );
});

test("home badge thread reply read state includes per-message markers", () => {
  const item = {
    ...feedItem("reply-1"),
    channelId: "stream-channel",
    createdAt: 500,
    tags: ROOT_TAGS,
  };

  const readAt = resolveHomeBadgeFeedItemReadAt(item, {
    getChannelReadAt: () => 300,
    getThreadReadAt: () => null,
    getMessageReadAt: (messageId) => (messageId === "reply-1" ? 500 : null),
  });

  assert.equal(readAt, 500);
  assert.equal(
    isHomeBadgeFeedItemUnread(item, {
      getChannelReadAt: () => 300,
      getThreadReadAt: () => null,
      getMessageReadAt: () => 500,
      seenFeedIdSet: new Set(),
    }),
    false,
  );
});

test("home badge thread reply read state uses newest channel thread or message marker", () => {
  const item = {
    ...feedItem("reply-1"),
    channelId: "stream-channel",
    createdAt: 500,
    tags: ROOT_TAGS,
  };

  assert.equal(
    resolveHomeBadgeFeedItemReadAt(item, {
      getChannelReadAt: () => 300,
      getThreadReadAt: () => 550,
      getMessageReadAt: () => 400,
    }),
    550,
  );
});

test("home badge subtotal counts locally unread rows before channel exclusion", () => {
  const highPriorityChannelIds = new Set(["stream-channel"]);

  assert.equal(
    shouldCountTowardHomeBadgeSubtotal(
      { channelId: "stream-channel", channelType: "stream", tags: [] },
      highPriorityChannelIds,
      true,
    ),
    true,
  );
});

test("notification settings default to sound on", () => {
  assert.equal(sanitizeNotificationSettings(undefined).soundEnabled, true);
  assert.equal(
    sanitizeNotificationSettings("not an object").soundEnabled,
    true,
  );
});

test("stored settings without a soundEnabled key read as sound on", () => {
  // Settings persisted before the switch existed carry no `soundEnabled`;
  // they must keep chiming rather than go silent on upgrade.
  const legacy = {
    desktopEnabled: true,
    homeBadgeEnabled: false,
    slotAlertsEnabled: { dm: false },
  };

  const settings = sanitizeNotificationSettings(legacy);

  assert.equal(settings.soundEnabled, true);
  assert.equal(settings.homeBadgeEnabled, false);
  assert.equal(settings.slotAlertsEnabled.dm, false);
});

test("a stored soundEnabled of false round-trips", () => {
  const stored = JSON.parse(
    JSON.stringify({
      ...sanitizeNotificationSettings(undefined),
      soundEnabled: false,
    }),
  );

  assert.equal(sanitizeNotificationSettings(stored).soundEnabled, false);
  assert.equal(
    sanitizeNotificationSettings({ soundEnabled: "no" }).soundEnabled,
    true,
  );
});

test("the retired slotAlertsSnapshot key is read past and no longer persisted", () => {
  // Settings saved by the old master switch carry a snapshot of the rows it
  // bulk-disabled. The sanitizer must tolerate it and drop it from the result.
  const settings = sanitizeNotificationSettings({
    slotAlertsEnabled: { dm: false },
    slotAlertsSnapshot: { dm: true, mention: true },
  });

  assert.equal(settings.slotAlertsEnabled.dm, false);
  assert.equal("slotAlertsSnapshot" in settings, false);
});
