import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";

import {
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "./channelWindowStore.ts";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys.ts";
import { pageOlderMessagesUntilRowFloor } from "./pageOlderMessages.ts";

function event(id, createdAt) {
  return {
    id: id.padEnd(64, "0"),
    pubkey: "a".repeat(64),
    created_at: createdAt,
    kind: 9,
    tags: [["h", "channel"]],
    content: id,
    sig: "b".repeat(128),
  };
}
const cursor = (item) => ({ createdAt: item.created_at, eventId: item.id });
function page(startCursor, rows, hasMore = true) {
  return {
    startCursor,
    rows: rows.map((item) => ({ event: item, thread: null })),
    aux: [],
    nextCursor: hasMore ? cursor(rows.at(-1)) : null,
    hasMore,
  };
}
function harness(channelId) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const head = page(null, [event(`${channelId}-head`, 300)]);
  queryClient.setQueryData(
    channelWindowKey(channelId),
    replaceNewestChannelWindow(emptyChannelWindowStore(), head),
  );
  queryClient.setQueryData(channelMessagesKey(channelId), [head.rows[0].event]);
  return { queryClient, head };
}
const flush = () => new Promise((resolve) => setImmediate(resolve));

test("commits one page and stages exactly one successor outside projection", async () => {
  const channelId = "stage-successor";
  const { queryClient, head } = harness(channelId);
  const older = page(head.nextCursor, [event("older", 200)]);
  const successor = page(older.nextCursor, [event("successor", 100)], false);
  const calls = [];
  const fetchPage = async (_channelId, requestCursor) => {
    calls.push(requestCursor);
    return calls.length === 1 ? older : successor;
  };

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  await flush();

  const store = queryClient.getQueryData(channelWindowKey(channelId));
  assert.deepEqual(calls, [head.nextCursor, older.nextCursor]);
  assert.equal(store.pages.length, 2);
  assert.equal(store.stagedPage, successor);
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey(channelId))
      .map((item) => item.content),
    ["older", `${channelId}-head`],
  );
});

test("trigger during successor staging awaits the same request", async () => {
  const channelId = "await-in-flight-stage";
  const { queryClient, head } = harness(channelId);
  const older = page(head.nextCursor, [event("older", 200)]);
  const successor = page(older.nextCursor, [event("successor", 100)], false);
  let releaseSuccessor;
  const successorPending = new Promise((resolve) => {
    releaseSuccessor = () => resolve(successor);
  });
  const calls = [];
  const fetchPage = async (_channelId, requestCursor) => {
    calls.push(requestCursor);
    return calls.length === 1 ? older : successorPending;
  };

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  const secondPass = pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  await flush();

  assert.deepEqual(calls, [head.nextCursor, older.nextCursor]);
  releaseSuccessor();
  await secondPass;
  assert.equal(
    queryClient.getQueryData(channelWindowKey(channelId)).pages.length,
    3,
  );
});

test("replaced tail does not reuse its stale in-flight stage", async () => {
  const channelId = "reject-stale-in-flight-stage";
  const { queryClient, head } = harness(channelId);
  const older = page(head.nextCursor, [event("older", 200)]);
  const staleSuccessor = page(
    older.nextCursor,
    [event("stale-successor", 100)],
    false,
  );
  const freshSuccessor = page(
    older.nextCursor,
    [event("fresh-successor", 90)],
    false,
  );
  let releaseStaleSuccessor;
  const staleSuccessorPending = new Promise((resolve) => {
    releaseStaleSuccessor = () => resolve(staleSuccessor);
  });
  const calls = [];
  const fetchPage = async (_channelId, requestCursor) => {
    calls.push(requestCursor);
    if (calls.length === 1) return older;
    if (calls.length === 2) return staleSuccessorPending;
    return freshSuccessor;
  };

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  const replacementHead = page(null, [older.rows[0].event]);
  queryClient.setQueryData(
    channelWindowKey(channelId),
    replaceNewestChannelWindow(
      queryClient.getQueryData(channelWindowKey(channelId)),
      replacementHead,
    ),
  );

  const replacementPass = pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  try {
    await flush();
    assert.deepEqual(calls, [
      head.nextCursor,
      older.nextCursor,
      replacementHead.nextCursor,
    ]);
  } finally {
    releaseStaleSuccessor();
  }
  await replacementPass;
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey(channelId))
      .map((item) => item.content),
    ["fresh-successor", "older"],
  );

  await flush();
  assert.equal(
    queryClient.getQueryData(channelWindowKey(channelId)).stagedPage,
    null,
  );
});

test("next paging pass consumes the staged page without network wait", async () => {
  const channelId = "consume-staged";
  const { queryClient, head } = harness(channelId);
  const older = page(head.nextCursor, [event("older", 200)]);
  const successor = page(older.nextCursor, [event("successor", 100)], false);
  let calls = 0;
  const fetchPage = async () => {
    calls += 1;
    return calls === 1 ? older : successor;
  };

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    fetchPage,
  );
  await flush();
  assert.equal(calls, 2);

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    channelId,
    () => true,
    () => {
      throw new Error("staged page should make this pass network-free");
    },
  );

  const store = queryClient.getQueryData(channelWindowKey(channelId));
  assert.equal(store.pages.length, 3);
  assert.equal(store.stagedPage, null);
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey(channelId))
      .map((item) => item.content),
    ["successor", "older", `${channelId}-head`],
  );
});
