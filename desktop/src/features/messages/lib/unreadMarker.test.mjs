import assert from "node:assert/strict";
import test from "node:test";

import {
  computeChannelUnreadMarker,
  computeThreadUnreadMarker,
} from "./unreadMarker.ts";

function topLevel(id, createdAt) {
  return { id, createdAt, author: "a", time: "", body: "", depth: 0 };
}

function reply(id, createdAt, parentId) {
  return { id, createdAt, author: "a", time: "", body: "", depth: 1, parentId };
}

test("computeChannelUnreadMarker_emptyTimeline_returnsNoUnread", () => {
  const marker = computeChannelUnreadMarker([], 100);
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeChannelUnreadMarker_nullFrontier_marksEveryTopLevelUnread", () => {
  const messages = [topLevel("a", 10), topLevel("b", 20), topLevel("c", 30)];
  const marker = computeChannelUnreadMarker(messages, null);
  assert.equal(marker.firstUnreadMessageId, "a");
  assert.equal(marker.unreadCount, 3);
});

test("computeChannelUnreadMarker_frontierBelowFirst_allUnread", () => {
  const messages = [topLevel("a", 10), topLevel("b", 20)];
  const marker = computeChannelUnreadMarker(messages, 5);
  assert.equal(marker.firstUnreadMessageId, "a");
  assert.equal(marker.unreadCount, 2);
});

test("computeChannelUnreadMarker_frontierBetweenMessages_marksOldestAfterFrontier", () => {
  const messages = [topLevel("a", 10), topLevel("b", 20), topLevel("c", 30)];
  const marker = computeChannelUnreadMarker(messages, 15);
  assert.equal(marker.firstUnreadMessageId, "b");
  assert.equal(marker.unreadCount, 2);
});

test("computeChannelUnreadMarker_frontierAtMessageTimestamp_isInclusive", () => {
  // A message whose createdAt equals the frontier is considered read
  // (strictly greater-than is unread), matching the read-marker semantics.
  const messages = [topLevel("a", 10), topLevel("b", 20)];
  const marker = computeChannelUnreadMarker(messages, 20);
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeChannelUnreadMarker_frontierAtLatest_returnsNoUnread", () => {
  const messages = [topLevel("a", 10), topLevel("b", 20)];
  const marker = computeChannelUnreadMarker(messages, 100);
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeChannelUnreadMarker_threadRepliesExcluded_onlyTopLevelCounted", () => {
  // Thread replies (with parentId) are out of scope for the channel divider.
  const messages = [
    topLevel("root", 10),
    reply("r1", 25, "root"),
    topLevel("b", 30),
  ];
  const marker = computeChannelUnreadMarker(messages, 15);
  assert.equal(marker.firstUnreadMessageId, "b");
  assert.equal(marker.unreadCount, 1);
});

test("computeChannelUnreadMarker_unreadAfterReadReplies_picksTopLevel", () => {
  // A newer reply does not become the divider target even if it is unread.
  const messages = [topLevel("a", 10), topLevel("b", 20), reply("r1", 50, "a")];
  const marker = computeChannelUnreadMarker(messages, 15);
  assert.equal(marker.firstUnreadMessageId, "b");
  assert.equal(marker.unreadCount, 1);
});

test("computeChannelUnreadMarker_suppressed_returnsNoMarkerDespiteUnread", () => {
  // Manually marking the channel unread suppresses the in-timeline marker so
  // the pill/divider do not contradict the sidebar dot. Messages that would
  // otherwise be unread (frontier below them) produce nothing when suppressed.
  const messages = [topLevel("a", 10), topLevel("b", 20)];
  const marker = computeChannelUnreadMarker(messages, 5, true);
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeChannelUnreadMarker_suppressedNeverReadChannel_returnsNoMarker", () => {
  // Suppression overrides the never-read (null frontier) case too.
  const messages = [topLevel("a", 10), topLevel("b", 20)];
  const marker = computeChannelUnreadMarker(messages, null, true);
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

// --- computeThreadUnreadMarker tests ---

test("computeThreadUnreadMarker_emptyReplies_returnsNoUnread", () => {
  const marker = computeThreadUnreadMarker([], 100);
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_nullFrontier_marksAllRepliesUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, null);
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 3);
});

test("computeThreadUnreadMarker_frontierBetweenReplies_countsAfterFrontier", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, 15);
  assert.equal(marker.firstUnreadReplyId, "r2");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_frontierAtReplyTimestamp_isRead", () => {
  // A reply whose createdAt equals the frontier is considered read (strictly >).
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, 20);
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_frontierAboveAll_returnsNoUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, 100);
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_frontierBelowAll_allUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, 5);
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_singleReplyUnread_countsOne", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, 25);
  assert.equal(marker.firstUnreadReplyId, "r3");
  assert.equal(marker.unreadCount, 1);
});

test("computeThreadUnreadMarker_emptyRepliesNullFrontier_returnsNoUnread", () => {
  const marker = computeThreadUnreadMarker([], null);
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});
