import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";

import { channelWindowKey, threadRepliesKey } from "./messageQueryKeys.ts";
import { reconcileLiveThreadSummary } from "./threadSummaryReconciliation.ts";

const channelId = "channel";
const rootId = "r".repeat(64);

function event(id, { kind = 9, pending = false } = {}) {
  return {
    id: id.padEnd(64, "0"),
    pubkey: "a".repeat(64),
    created_at: 1_700_000_000,
    kind,
    tags: [["h", channelId]],
    content: id,
    sig: "b".repeat(128),
    ...(pending ? { pending: true } : {}),
  };
}

function summaryEvent(id, descendantCount, createdAt = 1_700_000_100) {
  return {
    ...event(id),
    created_at: createdAt,
    kind: 39005,
    tags: [
      ["h", channelId],
      ["e", rootId],
      ["d", rootId],
    ],
    content: JSON.stringify({
      reply_count: descendantCount,
      descendant_count: descendantCount,
      last_reply_at: createdAt - 1,
      participants: ["a".repeat(64)],
    }),
  };
}

function createHarness(replies) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const threadKey = threadRepliesKey(channelId, rootId);
  if (replies !== undefined) client.setQueryData(threadKey, replies);
  return { client, threadKey, windowKey: channelWindowKey(channelId) };
}

test("summary recount invalidates a loaded thread missing a descendant", () => {
  const harness = createHarness([event("one")]);

  assert.equal(
    reconcileLiveThreadSummary(
      harness.client,
      channelId,
      summaryEvent("summary", 2),
    ),
    true,
  );
  assert.equal(
    harness.client.getQueryState(harness.threadKey).isInvalidated,
    true,
  );
  assert.equal(
    harness.client.getQueryData(harness.windowKey).liveSummaries[rootId].summary
      .descendantCount,
    2,
  );
});

test("matching committed descendants do not invalidate for aux or optimistic rows", () => {
  const harness = createHarness([
    event("committed"),
    event("reaction", { kind: 7 }),
    event("edit", { kind: 40003 }),
    event("optimistic", { pending: true }),
  ]);

  reconcileLiveThreadSummary(
    harness.client,
    channelId,
    summaryEvent("summary", 1),
  );

  assert.equal(
    harness.client.getQueryState(harness.threadKey).isInvalidated,
    false,
  );
});

test("same-second winner can count down and invalidate after a delete", () => {
  const harness = createHarness([event("one"), event("two")]);
  const timestamp = 1_700_000_100;

  reconcileLiveThreadSummary(
    harness.client,
    channelId,
    summaryEvent("8".repeat(64), 2, timestamp),
  );
  assert.equal(
    harness.client.getQueryState(harness.threadKey).isInvalidated,
    false,
  );

  assert.equal(
    reconcileLiveThreadSummary(
      harness.client,
      channelId,
      summaryEvent("1".repeat(64), 1, timestamp),
    ),
    true,
  );
  assert.equal(
    harness.client.getQueryState(harness.threadKey).isInvalidated,
    true,
  );
  assert.equal(
    harness.client.getQueryData(harness.windowKey).liveSummaries[rootId]
      .eventId,
    "1".repeat(64),
  );

  // A higher id loses the same-second tie and cannot roll the winner back.
  assert.equal(
    reconcileLiveThreadSummary(
      harness.client,
      channelId,
      summaryEvent("f".repeat(64), 2, timestamp),
    ),
    false,
  );
  assert.equal(
    harness.client.getQueryData(harness.windowKey).liveSummaries[rootId].summary
      .descendantCount,
    1,
  );
});

test("replayed authoritative summary reconciles a cache loaded afterward", () => {
  const harness = createHarness(undefined);
  const summary = summaryEvent("summary", 2);

  reconcileLiveThreadSummary(harness.client, channelId, summary);
  assert.equal(harness.client.getQueryState(harness.threadKey), undefined);

  harness.client.setQueryData(harness.threadKey, [event("one")]);
  assert.equal(
    reconcileLiveThreadSummary(harness.client, channelId, summary),
    true,
  );
  assert.equal(
    harness.client.getQueryState(harness.threadKey).isInvalidated,
    true,
  );
});
