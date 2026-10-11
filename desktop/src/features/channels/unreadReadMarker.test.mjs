import assert from "node:assert/strict";
import test from "node:test";

import { computeChannelUnreadMarker } from "../messages/lib/unreadMarker.ts";
import {
  countUnreadAppBadgeObservedEvents,
  countUnreadBadgeObservedEvents,
  countUnreadHighPriorityObservedEvents,
  countUnreadObservedEvents,
  hasUnreadTopLevelObservedEvent,
  observedUnreadEventReadAt,
  recordObservedUnreadEvent,
} from "./unreadChannelCounts.ts";
import {
  addThreadActivityItems,
  channelCatchUpEventKinds,
  resolveChannelReadMarker,
  resolveObservedUnreadRootId,
} from "./useUnreadChannels.ts";
import {
  isChannelUnreadTriggerKind,
  trackSeenEvent,
  withChannelTagFallback,
} from "./useLiveChannelUpdates.ts";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_STARTED,
  KIND_STREAM_MESSAGE,
} from "@/shared/constants/kinds";

function topLevel(id, createdAt) {
  return { id, createdAt, author: "a", time: "", body: "", depth: 0 };
}

// The headline scenario the fix restores: messages arrive while the channel is
// inactive, the read frontier was captured before them, and on reopen the
// pill and divider must render. The deleted AppShell effect used to fold those
// just-arrived timestamps into the frontier, hiding them; with it gone the
// frontier stays below the new messages.
test("receiveThenReopen_frontierBelowArrivedMessages_showsDivider", () => {
  const frontierBeforeReceive = 100;
  const arrived = [
    topLevel("seen", 90),
    topLevel("new-1", 110),
    topLevel("new-2", 120),
  ];

  const marker = computeChannelUnreadMarker(arrived, frontierBeforeReceive);

  assert.equal(marker.firstUnreadMessageId, "new-1");
  assert.equal(marker.unreadCount, 2);
});

// Regression guard for the read frontier silently clobbering newly received
// messages: if the marker had advanced to the latest arrival (as the deleted
// effect did), nothing would be unread.
test("receiveThenReopen_frontierAtLatestArrival_clobbersDivider", () => {
  const arrived = [topLevel("a", 90), topLevel("b", 110), topLevel("c", 120)];

  const marker = computeChannelUnreadMarker(arrived, 120);

  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("live reaction without h tag inherits its subscription channel", () => {
  const reaction = {
    id: "reaction",
    kind: 7,
    pubkey: "agent",
    created_at: 1,
    content: "👀",
    tags: [["e", "message"]],
  };

  assert.deepEqual(withChannelTagFallback(reaction, "channel-a").tags, [
    ["e", "message"],
    ["h", "channel-a"],
  ]);
});

test("live event with h tag is preserved", () => {
  const message = {
    id: "message",
    kind: KIND_STREAM_MESSAGE,
    pubkey: "author",
    created_at: 1,
    content: "hello",
    tags: [["h", "channel-from-event"]],
  };

  assert.equal(withChannelTagFallback(message, "other-channel"), message);
});

test("notification event guard suppresses reconnect replay and stays bounded", () => {
  const seen = new Set();

  assert.equal(trackSeenEvent(seen, "event-a", 2), true);
  assert.equal(trackSeenEvent(seen, "event-a", 2), false);
  assert.equal(trackSeenEvent(seen, "event-b", 2), true);
  assert.equal(trackSeenEvent(seen, "event-c", 2), true);
  assert.deepEqual([...seen], ["event-b", "event-c"]);
});

test("dmHuddleStart_isDmOnlyUnreadTrigger", () => {
  assert.equal(
    isChannelUnreadTriggerKind(KIND_HUDDLE_STARTED, true),
    true,
    "inactive DM huddle start should bump unread",
  );
  assert.equal(
    isChannelUnreadTriggerKind(KIND_HUDDLE_STARTED, false),
    false,
    "stream/forum huddle start should not become a generic unread trigger",
  );
  assert.equal(
    isChannelUnreadTriggerKind(KIND_HUDDLE_ENDED, true),
    false,
    "huddle end lifecycle should stay quiet",
  );
});

test("dmCatchUpFetch_includesOnlyHuddleStartInvite", () => {
  const dmKinds = channelCatchUpEventKinds("dm");
  const streamKinds = channelCatchUpEventKinds("stream");

  assert.equal(dmKinds.includes(KIND_STREAM_MESSAGE), true);
  assert.equal(dmKinds.includes(KIND_HUDDLE_STARTED), true);
  assert.equal(dmKinds.includes(KIND_HUDDLE_ENDED), false);
  assert.equal(streamKinds.includes(KIND_HUDDLE_STARTED), false);
});

// An explicit caller timeline position must still advance the read marker. This
// is the consumer (ChannelScreen) that marks the active channel read with a
// real position; the fix must not regress it.
test("resolveChannelReadMarker_realReadAt_advancesMarker", () => {
  const readAt = "2026-06-12T00:00:00.000Z";
  const expected = Math.floor(Date.parse(readAt) / 1000);

  const result = resolveChannelReadMarker(readAt, undefined);

  assert.equal(result.markAt, expected);
  assert.equal(result.clearObserved, false);
});

// The Esc-to-mark-read shortcut and sidebar mark-read pass a null/stale caller
// value and rely on the observed-latest fold to mark the channel read. The
// rejected in-function null-guard would have returned markAt === null here,
// silently no-opping those user actions. This proves the fold survives.
test("resolveChannelReadMarker_nullCallerWithObservedLatest_marksViaObserved", () => {
  const observedLatest = 200;

  const result = resolveChannelReadMarker(null, observedLatest);

  assert.equal(result.markAt, observedLatest);
  assert.equal(result.clearObserved, true);
});

// With no caller value and nothing observed there is nothing to mark; the
// marker resolves to null so markChannelRead short-circuits without writing.
test("resolveChannelReadMarker_noCallerNoObserved_returnsNull", () => {
  const result = resolveChannelReadMarker(null, undefined);

  assert.equal(result.markAt, null);
  assert.equal(result.clearObserved, false);
});

// --- Fix 2: sidebar badge evaluates each observed event against its own read context ---

test("resolveObservedUnreadRootId_treatsBroadcastReplyAsTopLevelUnread", () => {
  assert.equal(
    resolveObservedUnreadRootId([
      ["e", "root-1", "", "reply"],
      ["broadcast", "1"],
    ]),
    null,
  );
});

test("observedUnreadEventReadAt_unopenedThreadReplyUsesChannelMarker", () => {
  const event = observed("reply", 500, "root-1");

  const readAt = observedUnreadEventReadAt(event, 300, () => null);

  assert.equal(readAt, 300);
  assert.equal(event.createdAt > readAt, true);
});

test("observedUnreadEventReadAt_openedThreadReplyUsesThreadMarker", () => {
  const event = observed("reply", 500, "root-1");

  const readAt = observedUnreadEventReadAt(event, 300, (rootId) =>
    rootId === "root-1" ? 500 : null,
  );

  assert.equal(readAt, 500);
  assert.equal(event.createdAt > readAt, false);
});

test("observedUnreadEventReadAt_openedMessageReplyUsesMessageMarker", () => {
  const event = observed("reply", 500, "root-1");

  const readAt = observedUnreadEventReadAt(
    event,
    300,
    () => null,
    (messageId) => (messageId === "reply" ? 500 : null),
  );

  assert.equal(readAt, 500);
  assert.equal(event.createdAt > readAt, false);
});

test("observedUnreadEventReadAt_usesNewestAvailableMarker", () => {
  const event = observed("reply", 500, "root-1");

  assert.equal(
    observedUnreadEventReadAt(
      event,
      300,
      () => 450,
      () => 400,
    ),
    450,
  );
});

test("observedUnreadEventReadAt_topLevelUsesChannelMarker", () => {
  assert.equal(
    observedUnreadEventReadAt(observed("top", 500), 300, () => 900),
    300,
  );
});

test("observedUnreadEventReadAt_nullChannelMarkerThreadMarkerCanClear", () => {
  assert.equal(
    observedUnreadEventReadAt(
      observed("reply", 500, "root-1"),
      null,
      () => 500,
    ),
    500,
  );
});

// --- Fix 2b: sidebar badge evaluates all observed events, not a single aggregate frontier ---

function observed(
  id,
  createdAt,
  rootId = null,
  highPriority = false,
  countsTowardBadge = true,
  countsTowardAppBadge = countsTowardBadge,
) {
  return {
    id,
    createdAt,
    rootId,
    highPriority,
    countsTowardBadge,
    countsTowardAppBadge,
  };
}

function readAtFor(channelMarker, threadMarkers, messageMarkers = new Map()) {
  return (event) =>
    observedUnreadEventReadAt(
      event,
      channelMarker,
      (rootId) => threadMarkers.get(rootId) ?? null,
      (messageId) => messageMarkers.get(messageId) ?? null,
    );
}

test("countUnreadObservedEvents_clearsOpenedThreadButKeepsOtherUnreadThread", () => {
  // Channel marker is the newest top-level message (300). Two thread replies
  // arrived at 400 and 500. Opening root-newer writes thread:root-newer=500,
  // but root-older was never opened. The sidebar must stay unread because
  // root-older still has a reply newer than its own effective frontier.
  const events = new Map([
    ["older", observed("older", 400, "root-older")],
    ["newer", observed("newer", 500, "root-newer")],
  ]);
  const getReadAt = readAtFor(300, new Map([["root-newer", 500]]));

  assert.equal(countUnreadObservedEvents(events, getReadAt), 1);
});

test("sidebarPipeline_openThreadClearsOnlyUnreadThreadContribution", () => {
  const channelId = "chan";
  const rootId = "root-1";
  const reply = observed("reply", 500, rootId);
  const observedByChannel = new Map();
  recordObservedUnreadEvent(observedByChannel, channelId, reply, 20);

  // Channel-open advances only to the newest top-level message. Before the
  // thread is opened, the reply remains newer than the channel frontier, so the
  // sidebar badge is present.
  const beforeOpenReadAt = readAtFor(300, new Map());
  assert.equal(
    countUnreadObservedEvents(
      observedByChannel.get(channelId),
      beforeOpenReadAt,
    ),
    1,
  );

  // Thread-open writes the thread OWN marker. The sidebar recompute must check
  // the observed reply against that thread marker (not just the channel marker),
  // which clears the channel count for the reported scenario.
  const afterOpenReadAt = readAtFor(300, new Map([[rootId, 500]]));
  assert.equal(
    countUnreadObservedEvents(
      observedByChannel.get(channelId),
      afterOpenReadAt,
    ),
    0,
  );
});

test("sidebarPipeline_openedReplyMessageMarkerClearsDelayedReplay", () => {
  const channelId = "chan";
  const rootId = "root-1";
  const reply = observed("reply", 500, rootId);
  const observedByChannel = new Map();
  recordObservedUnreadEvent(observedByChannel, channelId, reply, 20);

  const beforeOpenReadAt = readAtFor(300, new Map(), new Map());
  assert.equal(
    countUnreadObservedEvents(
      observedByChannel.get(channelId),
      beforeOpenReadAt,
    ),
    1,
  );

  const afterOpenReadAt = readAtFor(300, new Map(), new Map([["reply", 500]]));
  assert.equal(
    countUnreadObservedEvents(
      observedByChannel.get(channelId),
      afterOpenReadAt,
    ),
    0,
  );
});

test("latestObservedEvent_latestThreadReadDoesNotImplyChannelClear", () => {
  const events = new Map([
    ["older", observed("older", 400, "root-older")],
    ["newer", observed("newer", 500, "root-newer")],
  ]);
  const getReadAt = readAtFor(300, new Map([["root-newer", 500]]));
  // This reproduces the bug in the rejected aggregate-frontier model:
  // checking only the latest event would clear the whole channel after reading
  // root-newer, even though root-older remains unread.
  const latestOnly = new Map([["newer", events.get("newer")]]);

  assert.equal(countUnreadObservedEvents(latestOnly, getReadAt), 0);
  assert.equal(countUnreadObservedEvents(events, getReadAt), 1);
});

test("countUnreadObservedEvents_topLevelUsesChannelMarker", () => {
  const events = new Map([
    ["top-old", observed("top-old", 250)],
    ["top-new", observed("top-new", 350)],
  ]);

  assert.equal(countUnreadObservedEvents(events, readAtFor(300, new Map())), 1);
});

test("hasUnreadTopLevelObservedEvent_ignoresUnreadThreadReplies", () => {
  const events = new Map([
    ["top-old", observed("top-old", 250)],
    ["thread-new", observed("thread-new", 500, "root-1")],
  ]);

  assert.equal(
    hasUnreadTopLevelObservedEvent(events, readAtFor(300, new Map())),
    false,
  );
});

test("hasUnreadTopLevelObservedEvent_detectsUnreadTopLevelMessage", () => {
  const events = new Map([
    ["top-new", observed("top-new", 350)],
    ["thread-new", observed("thread-new", 500, "root-1")],
  ]);

  assert.equal(
    hasUnreadTopLevelObservedEvent(events, readAtFor(300, new Map())),
    true,
  );
});

test("countUnreadBadgeObservedEvents_skipsBoldOnlyGeneralChannelItems", () => {
  const events = new Map([
    ["plain", observed("plain", 500, null, false, false)],
    ["thread", observed("thread", 600, "root-1")],
  ]);

  assert.equal(countUnreadObservedEvents(events, readAtFor(300, new Map())), 2);
  assert.equal(
    countUnreadBadgeObservedEvents(events, readAtFor(300, new Map())),
    1,
  );
  assert.equal(
    countUnreadAppBadgeObservedEvents(events, readAtFor(300, new Map())),
    1,
  );
});

test("countUnreadObservedEvents_countsThreadRepliesForChannelUnread", () => {
  const events = new Map([
    ["reply", observed("reply", 500, "root-1", false, true, false)],
  ]);

  assert.equal(countUnreadObservedEvents(events, readAtFor(300, new Map())), 1);
  assert.equal(
    countUnreadBadgeObservedEvents(events, readAtFor(300, new Map())),
    1,
  );
  assert.equal(
    countUnreadAppBadgeObservedEvents(events, readAtFor(300, new Map())),
    0,
  );
});

test("highPriorityObservedEvents_countsMentionBadgeForGeneralMessage", () => {
  const events = new Map([
    ["mention", observed("mention", 500, null, true, true)],
  ]);
  const getReadAt = readAtFor(300, new Map());

  assert.equal(countUnreadObservedEvents(events, getReadAt), 1);
  assert.equal(countUnreadBadgeObservedEvents(events, getReadAt), 1);
  assert.equal(countUnreadAppBadgeObservedEvents(events, getReadAt), 1);
  assert.equal(countUnreadHighPriorityObservedEvents(events, getReadAt), 1);
});

test("recordObservedUnreadEvent_reportsOutOfOrderInsertForInvalidation", () => {
  const channelId = "chan";
  const observedByChannel = new Map();

  assert.equal(
    recordObservedUnreadEvent(
      observedByChannel,
      channelId,
      observed("latest", 500, "root-latest"),
      20,
    ),
    true,
  );
  assert.equal(
    recordObservedUnreadEvent(
      observedByChannel,
      channelId,
      observed("older", 400, "root-older"),
      20,
    ),
    true,
  );
  assert.equal(
    recordObservedUnreadEvent(
      observedByChannel,
      channelId,
      observed("older", 400, "root-older"),
      20,
    ),
    false,
  );
  assert.equal(observedByChannel.get(channelId).size, 2);
});

test("highPriorityObservedEvents_countOnlyUnreadHighPriorityItems", () => {
  const events = new Map([
    ["mention-read", observed("mention-read", 500, "root-read", true)],
    ["normal-unread", observed("normal-unread", 600, "root-unread", false)],
    ["mention-unread", observed("mention-unread", 700, "root-hot", true)],
  ]);
  const getReadAt = readAtFor(
    300,
    new Map([
      ["root-read", 500],
      ["root-unread", 300],
      ["root-hot", 300],
    ]),
  );

  assert.equal(countUnreadObservedEvents(events, getReadAt), 2);
  assert.equal(countUnreadHighPriorityObservedEvents(events, getReadAt), 1);
});

test("addThreadActivityItems keeps newest items when input is newest-first", () => {
  const newestFirst = Array.from({ length: 101 }, (_, index) => {
    const createdAt = 100 - index;
    return {
      id: `reply-${createdAt}`,
      kind: 9,
      pubkey: "author",
      content: "reply",
      createdAt,
      channelId: "channel",
      channelName: "general",
      tags: [["h", "channel"]],
    };
  });

  const result = addThreadActivityItems([], newestFirst);

  assert.equal(result.didAdd, true);
  assert.equal(result.items.length, 100);
  assert.equal(result.items[0].id, "reply-1");
  assert.equal(result.items.at(-1).id, "reply-100");
});

// --- A future-dated created_at must not mark later messages read ---
//
// `created_at` is self-asserted by the sending client and the relay does not
// bound it for ordinary messages. Before the skew policy, one event dated a
// year out landed in the read marker verbatim, so every genuinely new message
// failed `createdAt > readAt` and was classified read: no badge, no divider,
// no thread resume, until wall-clock time caught up.
//
// The tolerance decides whether a timestamp is *plausible*; it is not a value
// to clamp to. Clamping an outlier to `now + 120` would manufacture a read
// frontier two minutes into the future and hide every legitimate message that
// arrives before the clock reaches it. An implausible timestamp is discarded
// and the marker repaired to the present instead.

const NOW = 1_780_000_000;
const YEAR_AHEAD = NOW + 365 * 24 * 60 * 60;

test("resolveChannelReadMarker_futureCallerReadAt_repairsToThePresent", () => {
  const result = resolveChannelReadMarker(
    new Date(YEAR_AHEAD * 1_000).toISOString(),
    undefined,
    NOW,
  );

  assert.equal(result.markAt, NOW);
  assert.ok(
    result.markAt < YEAR_AHEAD,
    "a future-dated message must not push the marker past it",
  );
});

test("resolveChannelReadMarker_repairedMarker_doesNotHideTheNextTwoMinutes", () => {
  // The regression the ceiling-clamp version had: a correct message arriving a
  // second from now must still be unread against the repaired marker.
  const { markAt } = resolveChannelReadMarker(
    new Date(YEAR_AHEAD * 1_000).toISOString(),
    undefined,
    NOW,
  );

  const marker = computeChannelUnreadMarker(
    [topLevel("soon", NOW + 1)],
    markAt,
  );

  assert.equal(marker.firstUnreadMessageId, "soon");
  assert.equal(marker.unreadCount, 1);
});

test("resolveChannelReadMarker_futureObservedLatest_repairsAndKeepsObserved", () => {
  const result = resolveChannelReadMarker(null, YEAR_AHEAD, NOW);

  assert.equal(result.markAt, NOW);
  // The observed refs must survive: that event really is still unread, so
  // clearing them would drop the sidebar dot the policy exists to preserve.
  assert.equal(result.clearObserved, false);
});

test("resolveChannelReadMarker_keepsThePlausibleInputWhenTheOtherIsPoison", () => {
  // Discarding is per-input, not on the max: a real caller position must not be
  // thrown away just because the observed timestamp beside it is implausible.
  const realRead = NOW - 3_600;

  const result = resolveChannelReadMarker(
    new Date(realRead * 1_000).toISOString(),
    YEAR_AHEAD,
    NOW,
  );

  assert.equal(result.markAt, realRead);
  assert.equal(result.clearObserved, false);
});

test("resolveChannelReadMarker_afterRepair_aLaterRealMessageIsStillUnread", () => {
  // The end-to-end shape of the bug, through the same comparison the divider
  // uses: poison arrives, the channel is marked read, then a genuine message.
  const { markAt } = resolveChannelReadMarker(
    new Date(YEAR_AHEAD * 1_000).toISOString(),
    undefined,
    NOW,
  );
  const genuineMessage = topLevel("later", NOW + 300);

  const marker = computeChannelUnreadMarker([genuineMessage], markAt);

  assert.equal(marker.firstUnreadMessageId, "later");
  assert.equal(marker.unreadCount, 1);
});

test("resolveChannelReadMarker_ordinarySkewInsideTolerance_isKept", () => {
  // A sender 30s ahead of this machine is normal. Discarding that would move
  // the marker back to now and leave a message the user just read unread.
  const slightlyAhead = NOW + 30;

  const result = resolveChannelReadMarker(
    new Date(slightlyAhead * 1_000).toISOString(),
    undefined,
    NOW,
  );

  assert.equal(result.markAt, slightlyAhead);
});

test("resolveChannelReadMarker_pastReadAt_isUnaffectedByTheCeiling", () => {
  // Comfortably before NOW: the ceiling must only ever bite upwards.
  const readAt = new Date((NOW - 86_400) * 1_000).toISOString();
  const expected = NOW - 86_400;

  assert.equal(
    resolveChannelReadMarker(readAt, undefined, NOW).markAt,
    expected,
  );
  assert.equal(resolveChannelReadMarker(null, 200, NOW).markAt, 200);
  assert.equal(resolveChannelReadMarker(null, undefined, NOW).markAt, null);
});
