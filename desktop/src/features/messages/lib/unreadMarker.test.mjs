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

// LP4 v3: the thread marker now reads a per-message resolver instead of a
// single frontier. A uniform read-line at `seconds` (or null = never read)
// reproduces the old frontier semantics for the shared-boundary cases.
function uniformReadAt(seconds) {
  return () => seconds;
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
  const marker = computeThreadUnreadMarker([], uniformReadAt(100));
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_neverRead_marksAllRepliesUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(null));
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 3);
});

test("computeThreadUnreadMarker_readLineBetweenReplies_countsAfterLine", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(15));
  assert.equal(marker.firstUnreadReplyId, "r2");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_readAtEqualsReplyTimestamp_isRead", () => {
  // A reply whose createdAt equals its read marker is read (strictly >).
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(20));
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_readLineAboveAll_returnsNoUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(100));
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_readLineBelowAll_allUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(5));
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_perMessageMarkers_countOnlyUnreadReply", () => {
  // The point of the per-message resolver: reading r2 leaves r1 and r3
  // unread independently — no single frontier could express this.
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const readAt = (id) => (id === "r2" ? 20 : null);
  const marker = computeThreadUnreadMarker(replies, readAt);
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_singleReplyUnread_countsOne", () => {
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
    { id: "r3", createdAt: 30 },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(25));
  assert.equal(marker.firstUnreadReplyId, "r3");
  assert.equal(marker.unreadCount, 1);
});

test("computeThreadUnreadMarker_emptyRepliesNeverRead_returnsNoUnread", () => {
  const marker = computeThreadUnreadMarker([], uniformReadAt(null));
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_forcedUnread_overridesReadMarker", () => {
  // Session-local mark-unread: r1 is read by its marker but forced unread,
  // so it counts; the OR-overlay never clears an otherwise-unread reply.
  const replies = [
    { id: "r1", createdAt: 10 },
    { id: "r2", createdAt: 20 },
  ];
  const marker = computeThreadUnreadMarker(
    replies,
    uniformReadAt(100),
    undefined,
    (id) => id === "r1",
  );
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 1);
});

// --- Self-authored skip tests ---

test("computeChannelUnreadMarker_selfAuthored_skipsOwnMessages", () => {
  const messages = [
    { ...topLevel("a", 10), pubkey: "me" },
    { ...topLevel("b", 20), pubkey: "other" },
    { ...topLevel("c", 30), pubkey: "me" },
  ];
  const marker = computeChannelUnreadMarker(messages, 5, false, "me");
  assert.equal(marker.firstUnreadMessageId, "b");
  assert.equal(marker.unreadCount, 1);
});

test("computeChannelUnreadMarker_allSelfAuthored_returnsNoUnread", () => {
  const messages = [
    { ...topLevel("a", 10), pubkey: "me" },
    { ...topLevel("b", 20), pubkey: "me" },
  ];
  const marker = computeChannelUnreadMarker(messages, 5, false, "me");
  assert.equal(marker.firstUnreadMessageId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeChannelUnreadMarker_noPubkey_countsNormally", () => {
  // When currentPubkey is not provided, all messages count.
  const messages = [
    { ...topLevel("a", 10), pubkey: "me" },
    { ...topLevel("b", 20), pubkey: "other" },
  ];
  const marker = computeChannelUnreadMarker(messages, 5);
  assert.equal(marker.firstUnreadMessageId, "a");
  assert.equal(marker.unreadCount, 2);
});

test("computeThreadUnreadMarker_selfAuthored_skipsOwnReplies", () => {
  const replies = [
    { id: "r1", createdAt: 10, pubkey: "me" },
    { id: "r2", createdAt: 20, pubkey: "other" },
    { id: "r3", createdAt: 30, pubkey: "me" },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(5), "me");
  assert.equal(marker.firstUnreadReplyId, "r2");
  assert.equal(marker.unreadCount, 1);
});

test("computeThreadUnreadMarker_allSelfAuthored_returnsNoUnread", () => {
  const replies = [
    { id: "r1", createdAt: 10, pubkey: "me" },
    { id: "r2", createdAt: 20, pubkey: "me" },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(5), "me");
  assert.equal(marker.firstUnreadReplyId, null);
  assert.equal(marker.unreadCount, 0);
});

test("computeThreadUnreadMarker_noPubkey_countsNormally", () => {
  const replies = [
    { id: "r1", createdAt: 10, pubkey: "me" },
    { id: "r2", createdAt: 20, pubkey: "other" },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(5));
  assert.equal(marker.firstUnreadReplyId, "r1");
  assert.equal(marker.unreadCount, 2);
});

test("computeChannelUnreadMarker_selfAuthoredMixedCase_skipsOwnMessages", () => {
  // Identity and signer pubkeys differing only in hex case must still match.
  const messages = [
    { ...topLevel("a", 10), pubkey: "ABCDEF" },
    { ...topLevel("b", 20), pubkey: "other" },
  ];
  const marker = computeChannelUnreadMarker(messages, 5, false, "abcdef");
  assert.equal(marker.firstUnreadMessageId, "b");
  assert.equal(marker.unreadCount, 1);
});

test("computeThreadUnreadMarker_selfAuthoredMixedCase_skipsOwnReplies", () => {
  const replies = [
    { id: "r1", createdAt: 10, pubkey: "ABCDEF" },
    { id: "r2", createdAt: 20, pubkey: "other" },
  ];
  const marker = computeThreadUnreadMarker(replies, uniformReadAt(5), "abcdef");
  assert.equal(marker.firstUnreadReplyId, "r2");
  assert.equal(marker.unreadCount, 1);
});
