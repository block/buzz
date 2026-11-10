import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";
import {
  consumeHydratedChannel,
  hasPersistedHydratedChannel,
  hydrateChannelHeads,
} from "./channelHeadCache.ts";
import { channelMessagesKey } from "./messageQueryKeys.ts";
import {
  reconcileFetchedChannelWindow,
  useChannelMessagesQuery,
  useChannelSubscription,
} from "../hooks.ts";
import { relayClient } from "../../../shared/api/relayClient.ts";
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const channel = {
  id: "channel-a",
  name: "general",
  channelType: "stream",
  visibility: "open",
  description: "",
  topic: null,
  purpose: null,
  memberCount: 1,
  memberPubkeys: [],
  lastMessageAt: null,
  archivedAt: null,
  participants: [],
  participantPubkeys: [],
  isMember: true,
  ttlSeconds: null,
  ttlDeadline: null,
};
let channelWindowCalls = 0;
let channelWindowEvents = [];

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());
const channelId = "channel-a";
const root = {
  id: "a".repeat(64),
  pubkey: "b".repeat(64),
  created_at: 10,
  kind: 40002,
  tags: [["h", channelId]],
  content: "persisted",
  sig: "",
};
const replacement = {
  ...root,
  id: "c".repeat(64),
  created_at: 11,
  content: "relay",
};
function bounds() {
  return {
    id: "d".repeat(64),
    pubkey: "e".repeat(64),
    created_at: 12,
    kind: 39006,
    tags: [
      ["h", channelId],
      ["d", `${channelId}:head`],
    ],
    content: JSON.stringify({ has_more: false, next_cursor: null }),
    sig: "",
  };
}
function install(entries, { loadDelayMs = 0 } = {}) {
  channelWindowCalls = 0;
  channelWindowEvents = [replacement, bounds()];
  window.localStorage.clear();
  window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "channel_head_cache_load") {
        if (loadDelayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, loadDelayMs));
        }
        return entries;
      }
      if (command === "get_channel_window") {
        channelWindowCalls += 1;
        return channelWindowEvents;
      }
      return null;
    },
  };
}

async function mountChannelQuery(client) {
  const { renderHook, waitFor } = await import("@testing-library/react");
  const view = renderHook(() => useChannelMessagesQuery(channel), {
    wrapper: ({ children }) =>
      React.createElement(QueryClientProvider, { client }, children),
  });
  await waitFor(() => assert.equal(view.result.current.isSuccess, true));
  return view;
}
test("mount fetches cold and prefetched channels but consumes hydrated data", async () => {
  install([]);
  const coldClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const coldView = await mountChannelQuery(coldClient);
  assert.equal(channelWindowCalls, 1);

  install([]);
  const prefetchedClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  prefetchedClient.setQueryData(channelMessagesKey(channelId), [root], {
    updatedAt: 0,
  });
  const prefetchedView = await mountChannelQuery(prefetchedClient);
  assert.equal(channelWindowCalls, 1);

  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const hydratedClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  await hydrateChannelHeads(hydratedClient, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  const hydratedView = await mountChannelQuery(hydratedClient);
  assert.equal(channelWindowCalls, 0);
  assert.deepEqual(hydratedView.result.current.data, [root]);

  await hydratedClient.invalidateQueries({
    queryKey: channelMessagesKey(channelId),
    exact: true,
    refetchType: "active",
  });
  assert.equal(channelWindowCalls, 1);
  assert.deepEqual(hydratedClient.getQueryData(channelMessagesKey(channelId)), [
    replacement,
  ]);
  hydratedView.unmount();
  coldView.unmount();
  prefetchedView.unmount();
  coldClient.clear();
  prefetchedClient.clear();
  hydratedClient.clear();
});

test("hydrates stale data and consumes its mount gate once", async () => {
  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  assert.deepEqual(client.getQueryData(channelMessagesKey(channelId)), [root]);
  assert.equal(
    client.getQueryState(channelMessagesKey(channelId)).dataUpdatedAt,
    0,
  );
  assert.equal(consumeHydratedChannel(client, channelId), true);
  assert.equal(consumeHydratedChannel(client, channelId), false);
});
test("authoritative refresh deletes a vanished hydrated row", async () => {
  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  const next = reconcileFetchedChannelWindow(
    client,
    channelId,
    [replacement, bounds()],
    client.getQueryData(channelMessagesKey(channelId)),
    new AbortController().signal,
  );
  assert.deepEqual(
    next.map((e) => e.id),
    [replacement.id],
  );
});
test("drops malformed entries independently", async () => {
  install([
    { channelId: "bad", events: [root], savedAt: 1, lastVisitedAt: 2 },
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  assert.equal(client.getQueryData(channelMessagesKey("bad")), undefined);
  assert.deepEqual(client.getQueryData(channelMessagesKey(channelId)), [root]);
});

test("a channel mounted during a slow cache load still takes the hydrated path", async () => {
  // Carl/#6572 (1): the app no longer waits for the cache before mounting, so
  // the channel query must itself wait for the seed instead of racing it with
  // a cold relay fetch.
  install(
    [{ channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 }],
    { loadDelayMs: 150 },
  );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  void hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  const view = await mountChannelQuery(client);
  assert.equal(channelWindowCalls, 0);
  assert.deepEqual(view.result.current.data, [root]);
  view.unmount();
  client.clear();
});
test("a bounds-only persisted head is not hydrated", async () => {
  // Carl/#6572 (3): zero rows paint nothing, so the channel must take the
  // cold path and hold its skeleton rather than flash the empty-channel intro.
  install([{ channelId, events: [bounds()], savedAt: 1, lastVisitedAt: 1 }]);
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  assert.equal(client.getQueryData(channelMessagesKey(channelId)), undefined);
  assert.equal(hasPersistedHydratedChannel(client, channelId), false);
  const view = await mountChannelQuery(client);
  assert.equal(channelWindowCalls, 1);
  assert.deepEqual(view.result.current.data, [replacement]);
  view.unmount();
  client.clear();
});

test("a hydrated channel still revalidates when live subscription setup fails", async () => {
  // Carl/#6572 (2): the hydrated path skips get_channel_window on mount, so
  // the post-subscribe refresh is its only authoritative fetch. A rejected
  // subscribe must trigger it too, or the channel stays stale all session.
  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "subscribeToChannelLive", () =>
    Promise.reject(new Error("socket down")),
  );
  const consoleError = mock.method(console, "error", () => {});
  try {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    await hydrateChannelHeads(client, {
      pubkey: "f".repeat(64),
      relayUrl: "wss://relay",
    });
    const { renderHook, waitFor } = await import("@testing-library/react");
    const view = renderHook(
      () => {
        useChannelSubscription(channel);
        return useChannelMessagesQuery(channel);
      },
      {
        wrapper: ({ children }) =>
          React.createElement(QueryClientProvider, { client }, children),
      },
    );
    await waitFor(() => assert.equal(channelWindowCalls, 1));
    await waitFor(() =>
      assert.deepEqual(view.result.current.data, [replacement]),
    );
    view.unmount();
    client.clear();
  } finally {
    mock.restoreAll();
  }
  assert.ok(
    consoleError.mock.calls.some(
      (call) => call.arguments[0] === "Failed to subscribe to channel",
    ),
  );
});

test("a live subscription that settles before a slow cache load still revalidates", async () => {
  // Carl/#6572 re-review: the query is parked on the hydration gate with no
  // data yet, so an invalidation issued now dedupes onto that in-flight fetch
  // (cancelRefetch only cancels when data exists). The seed then lands and the
  // queryFn returns the persisted snapshot — zero authoritative fetches.
  install(
    [{ channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 }],
    { loadDelayMs: 150 },
  );
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "subscribeToChannelLive", () =>
    Promise.resolve(async () => {}),
  );
  try {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    void hydrateChannelHeads(client, {
      pubkey: "f".repeat(64),
      relayUrl: "wss://relay",
    });
    const { renderHook, waitFor } = await import("@testing-library/react");
    const view = renderHook(
      () => {
        useChannelSubscription(channel);
        return useChannelMessagesQuery(channel);
      },
      {
        wrapper: ({ children }) =>
          React.createElement(QueryClientProvider, { client }, children),
      },
    );
    await waitFor(() => assert.equal(channelWindowCalls, 1));
    await waitFor(() =>
      assert.deepEqual(view.result.current.data, [replacement]),
    );
    view.unmount();
    client.clear();
  } finally {
    mock.restoreAll();
  }
});

test("a cold channel with an immediate live subscription fetches the window once", async () => {
  // The post-subscribe refresh must still dedupe onto a cold relay fetch that
  // is already in flight; only the hydration-parked fetch needs sequencing.
  install([]);
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "subscribeToChannelLive", () =>
    Promise.resolve(async () => {}),
  );
  try {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { renderHook, waitFor } = await import("@testing-library/react");
    const view = renderHook(
      () => {
        useChannelSubscription(channel);
        return useChannelMessagesQuery(channel);
      },
      {
        wrapper: ({ children }) =>
          React.createElement(QueryClientProvider, { client }, children),
      },
    );
    await waitFor(() =>
      assert.deepEqual(view.result.current.data, [replacement]),
    );
    await waitFor(() => assert.equal(view.result.current.isFetching, false));
    assert.equal(channelWindowCalls, 1);
    view.unmount();
    client.clear();
  } finally {
    mock.restoreAll();
  }
});
