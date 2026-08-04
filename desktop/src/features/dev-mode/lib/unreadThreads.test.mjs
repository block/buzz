import assert from "node:assert/strict";
import test from "node:test";

import {
  findUnreadFamilyTarget,
  selectUnreadThreadRoots,
} from "./unreadThreads.ts";

function summary(lastReplyAt) {
  return {
    replyCount: 1,
    descendantCount: 1,
    lastReplyAt,
    participantPubkeys: [],
  };
}

function replyItem(channelId, rootId, createdAt) {
  return {
    channelId,
    createdAt,
    tags: [
      ["e", rootId, "", "root"],
      ["e", rootId, "", "reply"],
    ],
  };
}

test("selectUnreadThreadRoots_flagsRepliesPastTheReadFrontier", () => {
  const summaries = new Map([
    ["seen", summary(100)],
    ["unseen", summary(200)],
    ["never-read", summary(50)],
    ["no-replies", summary(null)],
  ]);
  const readAts = new Map([
    ["seen", 100],
    ["unseen", 150],
  ]);
  const unread = selectUnreadThreadRoots(
    summaries,
    (rootId) => readAts.get(rootId) ?? null,
  );
  assert.deepEqual([...unread].sort(), ["never-read", "unseen"]);
});

test("findUnreadFamilyTarget_picksTheNewestUnreadThreadAcrossTheFamily", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main", "sub-a", "sub-b"],
    unreadChannelIds: new Set(["sub-a", "sub-b"]),
    topLevelUnreadChannelIds: new Set(),
    threadActivity: [
      replyItem("sub-a", "root-1", 100),
      replyItem("sub-b", "root-2", 300),
      replyItem("elsewhere", "root-3", 900),
    ],
    getThreadReadAt: () => null,
  });
  assert.deepEqual(target, { channelId: "sub-b", rootId: "root-2" });
});

test("findUnreadFamilyTarget_skipsRepliesAlreadyRead", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main"],
    unreadChannelIds: new Set(["main"]),
    topLevelUnreadChannelIds: new Set(),
    threadActivity: [replyItem("main", "root-1", 100)],
    getThreadReadAt: () => 100,
  });
  assert.equal(target, null);
});

test("findUnreadFamilyTarget_skipsChannelsTheSharedModelConsidersRead", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main", "sub-a"],
    unreadChannelIds: new Set(),
    topLevelUnreadChannelIds: new Set(),
    threadActivity: [replyItem("sub-a", "root-1", 100)],
    getThreadReadAt: () => null,
  });
  assert.equal(target, null);
});

test("findUnreadFamilyTarget_fallsBackToTopLevelUnreadMainFirst", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main", "sub-a", "sub-b"],
    unreadChannelIds: new Set(["main", "sub-b"]),
    topLevelUnreadChannelIds: new Set(["sub-b", "main"]),
    threadActivity: [],
    getThreadReadAt: () => null,
  });
  assert.deepEqual(target, { channelId: "main", rootId: null });
});

test("findUnreadFamilyTarget_threadReplyOutranksTopLevelUnread", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main", "sub-a"],
    unreadChannelIds: new Set(["main", "sub-a"]),
    topLevelUnreadChannelIds: new Set(["main"]),
    threadActivity: [replyItem("sub-a", "root-1", 100)],
    getThreadReadAt: () => null,
  });
  assert.deepEqual(target, { channelId: "sub-a", rootId: "root-1" });
});

test("findUnreadFamilyTarget_returnsNullWhenNothingIsUnread", () => {
  const target = findUnreadFamilyTarget({
    familyChannelIds: ["main"],
    unreadChannelIds: new Set(),
    topLevelUnreadChannelIds: new Set(),
    threadActivity: [],
    getThreadReadAt: () => null,
  });
  assert.equal(target, null);
});
