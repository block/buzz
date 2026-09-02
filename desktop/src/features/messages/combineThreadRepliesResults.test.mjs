import assert from "node:assert/strict";
import test from "node:test";

import {
  combineThreadRepliesForRoots,
  collectNewlyRevealedInlineReplyIds,
  combineThreadRepliesResults,
} from "./useThreadReplies.ts";

const CHANNEL_A = "a".repeat(64);
const CHANNEL_B = "b".repeat(64);

function event(id, createdAt) {
  return {
    id,
    pubkey: "c".repeat(64),
    kind: 9,
    created_at: createdAt,
    content: "reply",
    tags: [],
    sig: "sig",
  };
}

function replyEvent(id, rootId, createdAt) {
  return {
    ...event(id, createdAt),
    tags: [
      ["e", rootId, "", "root"],
      ["e", rootId, "", "reply"],
    ],
  };
}

function ok(data) {
  return {
    data,
    isFetching: false,
    isPending: false,
    isError: false,
    error: null,
    refetch: () => {
      throw new Error("a successful subtree must not be refetched");
    },
  };
}

function failed(refetch) {
  return {
    data: undefined,
    isFetching: false,
    isPending: false,
    isError: true,
    error: new Error("subtree load failed"),
    refetch,
  };
}

function pending() {
  return {
    data: undefined,
    isFetching: true,
    isPending: true,
    isError: false,
    error: null,
    refetch: () => {},
  };
}

test("aggregates events across roots in chronological order", () => {
  const combined = combineThreadRepliesResults([
    ok([event(CHANNEL_A, 200)]),
    ok([event(CHANNEL_B, 100)]),
  ]);
  assert.deepEqual(
    combined.events.map((e) => e.created_at),
    [100, 200],
  );
  assert.equal(combined.isPending, false);
  assert.equal(combined.isError, false);
  assert.equal(combined.error, null);
});

test("a failed subtree surfaces aggregate error and never silently drops", () => {
  // The load-bearing contract: one failed root among successful roots must make
  // the aggregate report isError so the consumer can surface a failure instead
  // of presenting a partial transcript as complete.
  const combined = combineThreadRepliesResults([
    ok([event(CHANNEL_A, 100)]),
    failed(() => {}),
  ]);
  assert.equal(combined.isError, true);
  assert.ok(combined.error instanceof Error);
  // Successful rows still contribute their events (non-destructive).
  assert.equal(combined.events.length, 1);
});

test("isPending reflects any still-loading root", () => {
  const combined = combineThreadRepliesResults([ok([]), pending()]);
  assert.equal(combined.isPending, true);
});

test("refetch re-runs only the failed subtrees, not the successful ones", () => {
  let failedRefetched = 0;
  const combined = combineThreadRepliesResults([
    ok([event(CHANNEL_A, 100)]),
    failed(() => {
      failedRefetched += 1;
    }),
  ]);
  // ok().refetch throws if called, so a partial-success refetch that touched
  // every query would throw here; it must only touch the failed one.
  combined.refetch();
  assert.equal(failedRefetched, 1);
});

test("all-success aggregate reports no error", () => {
  const combined = combineThreadRepliesResults([ok([]), ok([])]);
  assert.equal(combined.isError, false);
  assert.equal(combined.error, null);
});

test("tracks pending, refreshing, and failed state for the owning root only", () => {
  const readyRoot = "ready";
  const pendingRoot = "pending";
  const refreshingRoot = "refreshing";
  const failedRoot = "failed";
  const refreshing = { ...ok([]), isFetching: true };
  const combined = combineThreadRepliesForRoots(
    [readyRoot, pendingRoot, refreshingRoot, failedRoot],
    [ok([]), pending(), refreshing, failed(() => {})],
  );

  assert.deepEqual([...combined.pendingRootIds], [pendingRoot]);
  assert.deepEqual(
    [...combined.fetchingRootIds],
    [pendingRoot, refreshingRoot],
  );
  assert.deepEqual([...combined.errorRootIds], [failedRoot]);
});

test("marks only the reply snapshot revealed by each expansion", () => {
  const rootId = "root";
  const firstReply = replyEvent("first", rootId, 100);
  const futureReply = replyEvent("future", rootId, 200);
  const empty = new Set();

  const refreshingReveal = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: new Set([rootId]),
    errorRootIds: empty,
    events: [firstReply],
    revealedRootIds: empty,
  });
  assert.deepEqual(refreshingReveal.messageIds, []);

  const initialReveal = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: empty,
    events: [firstReply],
    revealedRootIds: refreshingReveal.revealedRootIds,
  });
  assert.deepEqual(initialReveal.messageIds, [rootId, firstReply.id]);

  const liveUpdate = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: empty,
    events: [firstReply, futureReply],
    revealedRootIds: initialReveal.revealedRootIds,
  });
  assert.deepEqual(liveUpdate.messageIds, []);

  const collapsed = collectNewlyRevealedInlineReplyIds({
    rootIds: empty,
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: empty,
    events: [firstReply, futureReply],
    revealedRootIds: liveUpdate.revealedRootIds,
  });
  const reopened = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: empty,
    events: [firstReply, futureReply],
    revealedRootIds: collapsed.revealedRootIds,
  });
  assert.deepEqual(reopened.messageIds, [
    rootId,
    firstReply.id,
    futureReply.id,
  ]);
});

test("marks cached replies on refresh failure without freezing the snapshot", () => {
  const rootId = "root";
  const cachedReply = replyEvent("cached", rootId, 100);
  const refreshedReply = replyEvent("refreshed", rootId, 200);
  const empty = new Set();

  const failedRefresh = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: new Set([rootId]),
    events: [cachedReply],
    revealedRootIds: empty,
  });
  assert.deepEqual(failedRefresh.messageIds, [rootId, cachedReply.id]);
  assert.deepEqual([...failedRefresh.revealedRootIds], []);

  const retrySucceeded = collectNewlyRevealedInlineReplyIds({
    rootIds: new Set([rootId]),
    pendingRootIds: empty,
    fetchingRootIds: empty,
    errorRootIds: empty,
    events: [cachedReply, refreshedReply],
    revealedRootIds: failedRefresh.revealedRootIds,
  });
  assert.deepEqual(retrySucceeded.messageIds, [
    rootId,
    cachedReply.id,
    refreshedReply.id,
  ]);
  assert.deepEqual([...retrySucceeded.revealedRootIds], [rootId]);
});
