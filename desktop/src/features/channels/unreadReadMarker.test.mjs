import assert from "node:assert/strict";
import test from "node:test";

import { computeChannelUnreadMarker } from "../messages/lib/unreadMarker.ts";
import {
  countUnreadHighPriorityObservedEvents,
  countUnreadObservedEvents,
  observedUnreadEventReadAt,
  recordObservedUnreadEvent,
} from "./unreadChannelCounts.ts";
import {
  addThreadActivityItems,
  resolveChannelReadMarker,
  resolveObservedUnreadRootId,
} from "./useUnreadChannels.ts";

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

function observed(id, createdAt, rootId = null, highPriority = false) {
  return { id, createdAt, rootId, highPriority };
}

function readAtFor(channelMarker, threadMarkers) {
  return (event) =>
    observedUnreadEventReadAt(
      event,
      channelMarker,
      (rootId) => threadMarkers.get(rootId) ?? null,
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
