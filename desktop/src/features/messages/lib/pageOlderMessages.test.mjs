import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";
import {
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "./channelWindowStore.ts";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys.ts";
import {
  admitOlderPage,
  discardStagedOlderMessages,
  pageOlderMessagesUntilRowFloor,
  stageOlderMessages,
} from "./pageOlderMessages.ts";
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
    rows: rows.map((event) => ({ event, thread: null })),
    aux: [],
    nextCursor: hasMore ? cursor(rows.at(-1)) : null,
    hasMore,
  };
}
function seeded() {
  const queryClient = new QueryClient();
  const head = page(null, [event("head", 100)]);
  const store = replaceNewestChannelWindow(emptyChannelWindowStore(), head);
  queryClient.setQueryData(channelWindowKey("channel"), store);
  queryClient.setQueryData(channelMessagesKey("channel"), [head.rows[0].event]);
  return { queryClient, head };
}
test("admits the exact tail cursor into both caches", () => {
  const { queryClient, head } = seeded();
  const older = page(head.nextCursor, [event("older", 90)], false);
  assert.equal(
    admitOlderPage(queryClient, "channel", head.nextCursor, older),
    true,
  );
  assert.equal(
    queryClient.getQueryData(channelWindowKey("channel")).pages.length,
    2,
  );
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey("channel"))
      .map((item) => item.content),
    ["older", "head"],
  );
});
test("rejects a staged page after a head refresh moves the tail cursor", () => {
  const { queryClient, head } = seeded();
  const staged = page(head.nextCursor, [event("older", 90)], false);
  const refreshed = page(null, [event("new-head", 110)]);
  const refreshedStore = replaceNewestChannelWindow(
    queryClient.getQueryData(channelWindowKey("channel")),
    refreshed,
  );
  queryClient.setQueryData(channelWindowKey("channel"), refreshedStore);
  const before = queryClient.getQueryData(channelMessagesKey("channel"));
  assert.equal(
    admitOlderPage(queryClient, "channel", head.nextCursor, staged),
    false,
  );
  assert.deepEqual(
    queryClient.getQueryData(channelWindowKey("channel")),
    refreshedStore,
  );
  assert.equal(queryClient.getQueryData(channelMessagesKey("channel")), before);
});
test("rejects a staged page after another pager advances the chain", () => {
  const { queryClient, head } = seeded();
  assert.equal(
    admitOlderPage(
      queryClient,
      "channel",
      head.nextCursor,
      page(head.nextCursor, [event("winner", 95)]),
    ),
    true,
  );
  const retained = queryClient.getQueryData(channelWindowKey("channel"));
  const messages = queryClient.getQueryData(channelMessagesKey("channel"));
  assert.equal(
    admitOlderPage(
      queryClient,
      "channel",
      head.nextCursor,
      page(head.nextCursor, [event("loser", 90)], false),
    ),
    false,
  );
  assert.equal(queryClient.getQueryData(channelWindowKey("channel")), retained);
  assert.equal(
    queryClient.getQueryData(channelMessagesKey("channel")),
    messages,
  );
});
test("rejects admission when the channel window was evicted", () => {
  const { queryClient, head } = seeded();
  queryClient.removeQueries({
    queryKey: channelWindowKey("channel"),
    exact: true,
  });
  const messages = queryClient.getQueryData(channelMessagesKey("channel"));
  assert.equal(
    admitOlderPage(
      queryClient,
      "channel",
      head.nextCursor,
      page(head.nextCursor, [event("older", 90)], false),
    ),
    false,
  );
  assert.equal(
    queryClient.getQueryData(channelMessagesKey("channel")),
    messages,
  );
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

test("same channel and cursor staging shares one request", async () => {
  const { queryClient, head } = seeded();
  const request = deferred();
  const cursors = [];
  const fetchPage = (_channelId, requestedCursor) => {
    cursors.push(requestedCursor);
    return request.promise;
  };

  const first = stageOlderMessages(queryClient, "channel", fetchPage);
  const second = stageOlderMessages(queryClient, "channel", fetchPage);
  assert.equal(cursors.length, 1);
  request.resolve(page(head.nextCursor, [event("older", 90)], false));
  await Promise.all([first, second]);
  discardStagedOlderMessages("channel");
});

test("a superseding tail cursor replaces the staged entry", async () => {
  const { queryClient, head } = seeded();
  const firstRequest = deferred();
  const secondRequest = deferred();
  const requests = [];
  const fetchPage = (_channelId, requestedCursor) => {
    requests.push(requestedCursor);
    return requests.length === 1 ? firstRequest.promise : secondRequest.promise;
  };

  const first = stageOlderMessages(queryClient, "channel", fetchPage);
  const advanced = page(head.nextCursor, [event("advanced", 95)]);
  assert.equal(
    admitOlderPage(queryClient, "channel", head.nextCursor, advanced),
    true,
  );
  const second = stageOlderMessages(queryClient, "channel", fetchPage);
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[1], advanced.nextCursor);

  firstRequest.resolve(page(head.nextCursor, [event("stale", 90)], false));
  secondRequest.resolve(
    page(advanced.nextCursor, [event("current", 90)], false),
  );
  await Promise.all([first, second]);
  discardStagedOlderMessages("channel");
});

test("discard prevents a late staged page from being consumed", async () => {
  const { queryClient, head } = seeded();
  const stagedRequest = deferred();
  let fallbackRequests = 0;
  const staging = stageOlderMessages(
    queryClient,
    "channel",
    () => stagedRequest.promise,
  );
  discardStagedOlderMessages("channel");
  stagedRequest.resolve(page(head.nextCursor, [event("discarded", 90)], false));
  await staging;

  await pageOlderMessagesUntilRowFloor(
    queryClient,
    "channel",
    () => true,
    async (_channelId, requestedCursor) => {
      fallbackRequests += 1;
      return page(requestedCursor, [event("fallback", 90)], false);
    },
  );
  assert.equal(fallbackRequests, 1);
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey("channel"))
      .map((item) => item.content),
    ["fallback", "head"],
  );
});

test("staged failure causes exactly one sentinel fallback request", async () => {
  const { queryClient } = seeded();
  await stageOlderMessages(queryClient, "channel", async () => {
    throw new Error("staging failed");
  });
  let fallbackRequests = 0;
  await pageOlderMessagesUntilRowFloor(
    queryClient,
    "channel",
    () => true,
    async (_channelId, requestedCursor) => {
      fallbackRequests += 1;
      return page(requestedCursor, [event("fallback", 90)], false);
    },
  );
  assert.equal(fallbackRequests, 1);
});

test("exact staged success avoids a sentinel network request", async () => {
  const { queryClient, head } = seeded();
  let stagedRequests = 0;
  await stageOlderMessages(
    queryClient,
    "channel",
    async (_channelId, cursor) => {
      stagedRequests += 1;
      return page(cursor, [event("staged", 90)], false);
    },
  );
  let fallbackRequests = 0;
  await pageOlderMessagesUntilRowFloor(
    queryClient,
    "channel",
    () => true,
    async () => {
      fallbackRequests += 1;
      return page(head.nextCursor, [event("unexpected", 80)], false);
    },
  );
  assert.equal(stagedRequests, 1);
  assert.equal(fallbackRequests, 0);
  assert.deepEqual(
    queryClient
      .getQueryData(channelMessagesKey("channel"))
      .map((item) => item.content),
    ["staged", "head"],
  );
});
