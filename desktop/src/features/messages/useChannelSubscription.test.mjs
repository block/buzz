/**
 * Seam test for the thread-replies freshness gap that `useChannelSubscription`
 * closes on (re)subscribe.
 *
 * The thread-replies query carries a 30s `staleTime` so reopening a thread
 * without leaving the channel is a warm cache hit rather than a refetch storm.
 * That is only correct while the channel's live subscription is feeding appends
 * into the cache. When the user switches channels the subscription is disposed,
 * so replies emitted meanwhile never reach the cache -- reopening within the
 * window would render stale topology and unread state (the five failing
 * `thread-unread.spec.ts` cases). `useChannelSubscription` invalidates the
 * channel's `["thread-replies", channelId]` queries once the subscription
 * establishes, mirroring the channel window's "freshness alone is not a proof"
 * resubscribe refresh, so the next open refetches.
 *
 * This mounts the real hook against a stubbed relay client and asserts a warm
 * thread-replies cache is marked stale after the subscription resolves, and
 * again on a reconnect. Removing either `invalidateThreadReplies()` call fails
 * the corresponding assertion.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { relayClient } from "@/shared/api/relayClient.ts";
import { threadRepliesKey } from "./lib/messageQueryKeys.ts";
import { useChannelSubscription } from "./hooks.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const ROOT_ID = "1".repeat(64);

function installDom() {
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div></body></html>",
  );
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  return dom;
}

// Replace the relay singleton's network methods with resolved stubs so the
// subscribe effect runs its establish path without a websocket. Exposes the
// registered reconnect listener so the test can fire it on demand.
function stubRelayClient() {
  const original = {
    subscribeToChannelLive: relayClient.subscribeToChannelLive,
    subscribeToReconnects: relayClient.subscribeToReconnects,
    setVisibleChannelId: relayClient.setVisibleChannelId,
  };
  let reconnectListener = () => {};
  relayClient.subscribeToChannelLive = async () => async () => {};
  relayClient.subscribeToReconnects = (listener) => {
    reconnectListener = listener;
    return () => {};
  };
  relayClient.setVisibleChannelId = () => {};
  return {
    triggerReconnect: () => reconnectListener(),
    restore() {
      Object.assign(relayClient, original);
    },
  };
}

afterEach(() => {
  delete globalThis.window;
  delete globalThis.document;
  delete globalThis.HTMLElement;
  delete globalThis.IS_REACT_ACT_ENVIRONMENT;
});

async function mountSubscription() {
  const dom = installDom();
  const stub = stubRelayClient();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.mount();

  const queryKey = threadRepliesKey(CHANNEL_ID, ROOT_ID);
  const seedWarmCache = () =>
    queryClient.setQueryData(queryKey, [{ id: ROOT_ID }]);
  seedWarmCache();

  const channel = { id: CHANNEL_ID, channelType: "channel" };
  function Harness() {
    useChannelSubscription(channel);
    return null;
  }
  const root = createRoot(dom.window.document.getElementById("root"));
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Harness),
      ),
    );
  });
  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => setImmediate(resolve));
    });
  };
  // Let the async subscribe promise resolve and run its establish callback.
  await settle();

  const isStale = () =>
    queryClient.getQueryState(queryKey)?.isInvalidated === true;

  return {
    isStale,
    seedWarmCache,
    settle,
    triggerReconnect: stub.triggerReconnect,
    cleanup() {
      act(() => root.unmount());
      queryClient.unmount();
      stub.restore();
    },
  };
}

test("subscribing invalidates the channel's warm thread-replies cache", async () => {
  const h = await mountSubscription();
  try {
    assert.equal(
      h.isStale(),
      true,
      "thread-replies cache must be invalidated once the live subscription establishes, so a reopen refetches replies missed while the channel was inactive",
    );
  } finally {
    h.cleanup();
  }
});

test("reconnecting re-invalidates a warm thread-replies cache", async () => {
  const h = await mountSubscription();
  try {
    h.seedWarmCache();
    assert.equal(h.isStale(), false, "precondition: cache is warm again");
    h.triggerReconnect();
    await h.settle();
    assert.equal(
      h.isStale(),
      true,
      "a reconnect must re-invalidate thread replies: events may have landed while the socket was down",
    );
  } finally {
    h.cleanup();
  }
});
