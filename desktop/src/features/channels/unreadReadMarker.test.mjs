import assert from "node:assert/strict";
import test from "node:test";

import { computeChannelUnreadMarker } from "../messages/lib/unreadMarker.ts";
import { resolveChannelReadMarker } from "./useUnreadChannels.ts";

function topLevel(id, createdAt) {
  return { id, createdAt, author: "a", time: "", body: "", depth: 0 };
}

// The headline scenario the fix restores: messages arrive while the channel is
// inactive, the read frontier was captured before them, and on reopen the pill
// and divider must render. The deleted AppShell effect used to fold those
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
