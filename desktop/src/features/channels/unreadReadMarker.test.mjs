import assert from "node:assert/strict";
import test from "node:test";

import { computeChannelUnreadMarker } from "../messages/lib/unreadMarker.ts";
import { isThreadReply } from "../messages/lib/threading.ts";
import {
  buildChannelThreadRoots,
  channelUnreadFrontier,
} from "./unreadChannelCounts.ts";
import {
  addThreadActivityItems,
  resolveChannelReadMarker,
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

// --- Sidebar dot routing: only main-channel activity belongs to the channel dot ---

test("isThreadReply detects normal threaded replies", () => {
  assert.equal(
    isThreadReply([
      ["h", "chan-a"],
      ["e", "root-1", "", "root"],
      ["e", "parent-1", "", "reply"],
    ]),
    true,
  );
});

test("isThreadReply treats top-level and broadcast replies as channel activity", () => {
  assert.equal(isThreadReply([["h", "chan-a"]]), false);
  assert.equal(
    isThreadReply([
      ["h", "chan-a"],
      ["broadcast", "1"],
      ["e", "root-1", "", "reply"],
    ]),
    false,
  );
});

// --- Sidebar dot routing: per-thread read markers fold into the channel frontier ---

function replyItem(channelId, rootId) {
  return {
    id: `${channelId}:${rootId}:${Math.random()}`,
    channelId,
    tags: [["e", rootId, "", "root"]],
  };
}

// rootId for these fixtures is the "root"-marked e-tag.
const getRootId = (tags) =>
  tags.find((tag) => tag[0] === "e" && tag[3] === "root")?.[1] ?? null;

test("buildChannelThreadRoots_groupsRootsByChannel", () => {
  const items = [
    replyItem("chan-a", "root-1"),
    replyItem("chan-a", "root-1"), // dedup within a channel
    replyItem("chan-a", "root-2"),
    replyItem("chan-b", "root-3"),
  ];
  const map = buildChannelThreadRoots(items, getRootId);

  assert.deepEqual([...(map.get("chan-a") ?? [])].sort(), ["root-1", "root-2"]);
  assert.deepEqual([...(map.get("chan-b") ?? [])], ["root-3"]);
  assert.equal(map.has("chan-c"), false);
});

test("buildChannelThreadRoots_skipsItemsWithNoRoot", () => {
  const items = [{ id: "x", channelId: "chan-a", tags: [["p", "someone"]] }];
  const map = buildChannelThreadRoots(items, getRootId);

  assert.equal(map.size, 0);
});

test("channelUnreadFrontier_unopenedThreadReply_dotPersists", () => {
  const channelMarker = 300;
  const threadRoots = new Set(["root-1"]);
  const getThreadOwnMarker = () => null;

  const frontier = channelUnreadFrontier(
    channelMarker,
    threadRoots,
    getThreadOwnMarker,
  );

  const latest = 500;
  assert.equal(frontier, 300);
  assert.equal(latest > frontier, true);
});

test("channelUnreadFrontier_openedThreadReply_dotClears", () => {
  const channelMarker = 300;
  const threadRoots = new Set(["root-1"]);
  const getThreadOwnMarker = (rootId) => (rootId === "root-1" ? 500 : null);

  const frontier = channelUnreadFrontier(
    channelMarker,
    threadRoots,
    getThreadOwnMarker,
  );

  const latest = 500;
  assert.equal(frontier, 500);
  assert.equal(latest > frontier, false);
});

test("channelUnreadFrontier_noThreadRoots_usesChannelMarker", () => {
  assert.equal(
    channelUnreadFrontier(300, undefined, () => null),
    300,
  );
  assert.equal(
    channelUnreadFrontier(null, undefined, () => null),
    null,
  );
});

test("channelUnreadFrontier_nullChannelMarker_threadMarkerGoverns", () => {
  const frontier = channelUnreadFrontier(null, new Set(["root-1"]), () => 500);
  assert.equal(frontier, 500);
});

test("channelUnreadFrontier_takesMaxAcrossMultipleThreads", () => {
  const frontier = channelUnreadFrontier(
    100,
    new Set(["root-1", "root-2"]),
    (rootId) => (rootId === "root-1" ? 400 : 700),
  );
  assert.equal(frontier, 700);
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
