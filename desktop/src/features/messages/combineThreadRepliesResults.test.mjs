import assert from "node:assert/strict";
import test from "node:test";

import { combineThreadRepliesResults } from "./useThreadReplies.ts";

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

function ok(data) {
  return {
    data,
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
    isPending: false,
    isError: true,
    error: new Error("subtree load failed"),
    refetch,
  };
}

function pending() {
  return {
    data: undefined,
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
