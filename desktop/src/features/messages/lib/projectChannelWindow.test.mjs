import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";

import { reconcileFetchedChannelWindow } from "../hooks.ts";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys.ts";
import {
  appendOlderChannelWindow,
  emptyChannelWindowStore,
  flattenChannelWindowEvents,
  mergeLiveChannelWindowEvent,
  replaceNewestChannelWindow,
} from "./channelWindowStore.ts";
import {
  projectChannelWindowMessages,
  refreshChannelWindowMessages,
} from "./projectChannelWindow.ts";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation.ts";

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

function wirePage(rows) {
  return [
    ...rows,
    {
      ...event("bounds", 0),
      kind: 39006,
      tags: [["d", "channel:head"]],
      content: JSON.stringify({ has_more: false, next_cursor: null }),
    },
  ];
}

function newestPage(rows) {
  return {
    startCursor: null,
    rows: rows.map((event) => ({ event, thread: null })),
    aux: [],
    nextCursor: null,
    hasMore: false,
  };
}

function createHarness() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const channelId = "channel";
  const messagesKey = channelMessagesKey(channelId);
  const windowKey = channelWindowKey(channelId);
  const initial = event("initial", 100);
  client.setQueryData(
    windowKey,
    replaceNewestChannelWindow(
      emptyChannelWindowStore(),
      newestPage([initial]),
    ),
  );
  client.setQueryData(messagesKey, [initial]);

  return { channelId, client, messagesKey, windowKey };
}

function appendLiveEvent(harness, live) {
  const current = harness.client.getQueryData(harness.windowKey);
  const next = mergeLiveChannelWindowEvent(current, live);
  harness.client.setQueryData(harness.windowKey, next);
  projectChannelWindowMessages(harness.client, harness.channelId);
}

function beginRefetch(harness, fetchPage, afterWindowWrite) {
  let resolveStarted;
  const started = new Promise((resolve) => {
    resolveStarted = resolve;
  });
  const observer = new QueryObserver(harness.client, {
    queryKey: harness.messagesKey,
    queryFn: async () => {
      resolveStarted();
      const page = await fetchPage;
      const current = harness.client.getQueryData(harness.windowKey);
      const next = replaceNewestChannelWindow(current, page);
      const previousMessages = harness.client.getQueryData(harness.messagesKey);
      harness.client.setQueryData(harness.windowKey, next);
      afterWindowWrite?.();
      return reconcileChannelWindowMessages(next, previousMessages);
    },
  });
  const unsubscribe = observer.subscribe(() => {});
  return {
    started,
    done: refreshChannelWindowMessages(
      harness.client,
      harness.channelId,
    ).finally(unsubscribe),
  };
}

function contents(harness) {
  return harness.client
    .getQueryData(harness.messagesKey)
    .map((event) => event.content);
}

test("test_live_event_during_fetch_survives_refetch_projection", async () => {
  const harness = createHarness();
  const live = event("during-fetch", 110);
  let resolveFetch;
  const pendingPage = new Promise((resolve) => {
    resolveFetch = resolve;
  });
  const { started, done } = beginRefetch(harness, pendingPage);

  await started;
  appendLiveEvent(harness, live);
  resolveFetch(newestPage([event("initial", 100)]));
  await done;

  assert.deepEqual(contents(harness), ["initial", "during-fetch"]);
});

test("test_live_event_after_query_resolution_survives_refetch_projection", async () => {
  const harness = createHarness();
  const live = event("post-resolve", 110);
  const { started, done } = beginRefetch(
    harness,
    Promise.resolve(newestPage([event("initial", 100)])),
    () => queueMicrotask(() => appendLiveEvent(harness, live)),
  );

  await started;
  await done;

  assert.deepEqual(contents(harness), ["initial", "post-resolve"]);
});

test("test_projection_retains_pending_send_and_non_broadcast_thread_reply", async () => {
  const harness = createHarness();
  const pending = { ...event("pending", 110), pending: true };
  const threadReply = {
    ...event("thread-reply", 120),
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "parent", "", "reply"],
    ],
  };
  const window = harness.client.getQueryData(harness.windowKey);
  const projected = reconcileChannelWindowMessages(window, [
    pending,
    threadReply,
  ]);

  assert.deepEqual(
    projected.map((event) => event.content),
    ["initial", "pending", "thread-reply"],
  );
});

test("test_projection_replaces_pending_send_with_authoritative_event", () => {
  const harness = createHarness();
  const pending = { ...event("accepted", 110), pending: true };
  const accepted = { ...event("accepted", 110), id: "c".repeat(64) };
  const window = replaceNewestChannelWindow(
    harness.client.getQueryData(harness.windowKey),
    newestPage([accepted, event("initial", 100)]),
  );

  const projected = reconcileChannelWindowMessages(window, [pending]);

  assert.deepEqual(
    projected.map((event) => event.content),
    ["initial", "accepted"],
  );
  assert.equal(projected[1]?.id, accepted.id);
  assert.equal(projected[1]?.localKey, pending.id);
});

test("test_reconciliation_preserves_dense_second_window_order", () => {
  const first = {
    ...newestPage([event("a", 100), event("b", 100)]),
    nextCursor: { createdAt: 100, eventId: event("b", 100).id },
    hasMore: true,
  };
  const store = appendOlderChannelWindow(
    replaceNewestChannelWindow(emptyChannelWindowStore(), first),
    {
      ...newestPage([event("c", 100), event("z", 99)]),
      startCursor: first.nextCursor,
    },
  );

  assert.deepEqual(
    reconcileChannelWindowMessages(store, []).map((item) => item.content),
    ["z", "c", "b", "a"],
  );
  assert.deepEqual(
    reconcileChannelWindowMessages(store, []).map((item) => item.id),
    flattenChannelWindowEvents(store).map((item) => item.id),
  );
});

test("test_reconciliation_retains_identical_pending_sends", () => {
  const harness = createHarness();
  const first = {
    ...event("first-pending", 110),
    content: "hello",
    pending: true,
  };
  const second = {
    ...event("second-pending", 111),
    content: "hello",
    pending: true,
  };

  const projected = reconcileChannelWindowMessages(
    harness.client.getQueryData(harness.windowKey),
    [first, second],
  );

  assert.deepEqual(
    projected.map((item) => item.id),
    [event("initial", 100), first, second].map((item) => item.id),
  );
});

test("test_reconciliation_acknowledges_only_one_identical_pending_send", () => {
  const harness = createHarness();
  const first = {
    ...event("first-pending", 110),
    content: "hello",
    pending: true,
  };
  const second = {
    ...event("second-pending", 111),
    content: "hello",
    pending: true,
  };
  const accepted = {
    ...event("accepted", 110),
    content: "hello",
  };
  const window = replaceNewestChannelWindow(
    harness.client.getQueryData(harness.windowKey),
    newestPage([accepted, event("initial", 100)]),
  );

  const projected = reconcileChannelWindowMessages(window, [first, second]);

  assert.deepEqual(
    projected.map((item) => item.id),
    [event("initial", 100), accepted, second].map((item) => item.id),
  );
  assert.equal(projected[1]?.localKey, first.id);
});

test("test_live_projection_retains_pending_send_and_non_broadcast_thread_reply", () => {
  const harness = createHarness();
  const pending = { ...event("pending", 110), pending: true };
  const threadReply = {
    ...event("thread-reply", 120),
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "parent", "", "reply"],
    ],
  };
  harness.client.setQueryData(harness.messagesKey, [pending, threadReply]);

  appendLiveEvent(harness, event("live", 130));

  assert.deepEqual(contents(harness), [
    "initial",
    "pending",
    "thread-reply",
    "live",
  ]);
});

test("test_canceled_stale_fetch_cannot_overwrite_catch_up_window", async () => {
  const harness = createHarness();
  const requests = [];
  let resolveRequestStarted;
  let requestStarted = new Promise((resolve) => {
    resolveRequestStarted = resolve;
  });
  const observer = new QueryObserver(harness.client, {
    queryKey: harness.messagesKey,
    queryFn: async ({ signal }) => {
      const previousMessages = harness.client.getQueryData(harness.messagesKey);
      let resolveFetch;
      const fetch = new Promise((resolve) => {
        resolveFetch = resolve;
      });
      requests.push({ resolveFetch, signal });
      resolveRequestStarted();
      const events = await fetch;
      return reconcileFetchedChannelWindow(
        harness.client,
        harness.channelId,
        events,
        previousMessages,
        signal,
      );
    },
  });
  const unsubscribe = observer.subscribe(() => {});

  await requestStarted;
  requestStarted = new Promise((resolve) => {
    resolveRequestStarted = resolve;
  });
  const catchUp = refreshChannelWindowMessages(
    harness.client,
    harness.channelId,
  );
  await requestStarted;

  assert.equal(requests[0].signal.aborted, true);
  requests[1].resolveFetch(
    wirePage([event("gap", 110), event("initial", 100)]),
  );
  await catchUp;
  assert.deepEqual(contents(harness), ["initial", "gap"]);

  requests[0].resolveFetch(wirePage([event("initial", 100)]));
  await new Promise((resolve) => setImmediate(resolve));
  appendLiveEvent(harness, event("live", 120));

  assert.deepEqual(contents(harness), ["initial", "gap", "live"]);
  assert.deepEqual(
    flattenChannelWindowEvents(
      harness.client.getQueryData(harness.windowKey),
    ).map((item) => item.content),
    ["initial", "gap", "live"],
  );
  unsubscribe();
});

test("test_pageless_live_projection_preserves_cached_timeline", () => {
  const harness = createHarness();
  const cached = harness.client.getQueryData(harness.messagesKey);
  const pageless = emptyChannelWindowStore();
  harness.client.setQueryData(harness.windowKey, pageless);

  const next = mergeLiveChannelWindowEvent(
    harness.client.getQueryData(harness.windowKey),
    event("live", 110),
  );
  harness.client.setQueryData(harness.windowKey, next);
  projectChannelWindowMessages(harness.client, harness.channelId);

  assert.deepEqual(contents(harness), ["initial", "live"]);
  assert.equal(harness.client.getQueryData(harness.messagesKey)[0], cached[0]);
});

test("test_concurrent_refreshes_after_seeded_snapshot_share_one_authoritative_fetch", async () => {
  // Subscribe settlement and a reconnect can both call the helper while the
  // channel query is still parked on its hydration-seeded snapshot. Both wake
  // on the same promise; the second invalidation must join the first
  // authoritative fetch, not cancel and replace it (Max/Wren, #6572 review).
  const harness = createHarness();
  const seeded = event("seeded", 100);
  harness.client.setQueryData(harness.messagesKey, [seeded], { updatedAt: 0 });
  const requests = [];
  const observer = new QueryObserver(harness.client, {
    queryKey: harness.messagesKey,
    queryFn: async ({ signal }) => {
      const previousMessages = harness.client.getQueryData(harness.messagesKey);
      let resolveFetch;
      const fetch = new Promise((resolve) => {
        resolveFetch = resolve;
      });
      requests.push({ resolveFetch, signal });
      const events = await fetch;
      return reconcileFetchedChannelWindow(
        harness.client,
        harness.channelId,
        events,
        previousMessages,
        signal,
      );
    },
  });
  const unsubscribe = observer.subscribe(() => {});
  try {
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(requests.length, 1);
    const first = refreshChannelWindowMessages(
      harness.client,
      harness.channelId,
    );
    const second = refreshChannelWindowMessages(
      harness.client,
      harness.channelId,
    );
    requests[0].resolveFetch(wirePage([seeded]));
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(requests.length, 2);
    requests[1].resolveFetch(wirePage([event("gap", 110), seeded]));
    await Promise.all([first, second]);
    assert.equal(requests.length, 2);
    assert.equal(requests[1].signal.aborted, false);
    assert.deepEqual(contents(harness), ["seeded", "gap"]);
  } finally {
    unsubscribe();
  }
});

test("test_subscription_refresh_preserves_cold_history_error", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const channelId = "cold-failure";
  const queryKey = channelMessagesKey(channelId);
  const observer = new QueryObserver(client, {
    queryKey,
    queryFn: async () => {
      throw new Error("history unavailable");
    },
  });
  const unsubscribe = observer.subscribe(() => {});

  try {
    await observer.refetch();
    assert.equal(observer.getCurrentResult().status, "error");
    assert.equal(observer.getCurrentResult().data, undefined);

    await assert.rejects(
      refreshChannelWindowMessages(client, channelId),
      /history unavailable/,
    );
    assert.equal(observer.getCurrentResult().status, "error");
    assert.equal(observer.getCurrentResult().data, undefined);
  } finally {
    unsubscribe();
  }
});

test("test_refresh_failure_retains_cached_rows_and_success_clears_error", async () => {
  const harness = createHarness();
  let shouldFail = true;
  const refreshed = event("refreshed", 110);
  const observer = new QueryObserver(harness.client, {
    queryKey: harness.messagesKey,
    queryFn: async ({ signal }) => {
      if (shouldFail) {
        throw new Error("history unavailable");
      }
      const previousMessages =
        harness.client.getQueryData(harness.messagesKey) ?? [];
      return reconcileFetchedChannelWindow(
        harness.client,
        harness.channelId,
        wirePage([refreshed, event("initial", 100)]),
        previousMessages,
        signal,
      );
    },
    staleTime: Number.POSITIVE_INFINITY,
  });
  const unsubscribe = observer.subscribe(() => {});

  try {
    await assert.rejects(
      refreshChannelWindowMessages(harness.client, harness.channelId),
      /history unavailable/,
    );
    assert.equal(observer.getCurrentResult().status, "error");
    assert.deepEqual(contents(harness), ["initial"]);

    shouldFail = false;
    await refreshChannelWindowMessages(harness.client, harness.channelId);
    assert.equal(observer.getCurrentResult().status, "success");
    assert.deepEqual(contents(harness), ["initial", "refreshed"]);
  } finally {
    unsubscribe();
  }
});
