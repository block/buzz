/**
 * Regression for the High finding: a live threaded reply must never make an
 * *absent* thread-replies cache key fresh.
 *
 * `appendMessage` writes each live threaded reply into the channel's
 * thread-replies cache so a warm thread stays current on reopen under the 30s
 * `staleTime`. The trap: if that write *creates* the key for a thread that was
 * never opened, React Query treats the one-row cache as complete/fresh, and the
 * next `useThreadReplies` mount skips `loadThreadReplies` — the thread renders
 * only the newest live reply and silently drops its entire older subtree.
 *
 * This drives the real producer (`useChannelSubscription.appendMessage`) against
 * a real QueryClient (a stub client cannot exercise `staleTime`), emits a live
 * reply for a never-opened thread, then models the thread open with
 * `fetchQuery` using the exact key/queryFn/staleTime `useThreadReplies` uses.
 * It asserts the open still runs the history fetch and unions the live reply
 * without loss or duplication. Reverting the producer to a create-capable
 * `setQueryData` makes `fetchQuery` observe a fresh key and skip the fetch,
 * failing the `fetchCount === 1` assertion.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import { threadRepliesKey } from "./lib/messageQueryKeys.ts";
import { useChannelSubscription } from "./hooks.ts";
import {
  THREAD_REPLIES_STALE_TIME_MS,
  loadThreadReplies,
} from "./useThreadReplies.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const ROOT_ID = "a".repeat(64);
const HISTORY_REPLY_ID = "c".repeat(64);
const LIVE_REPLY_ID = "1".repeat(64);

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

function relayEvent(overrides) {
  return {
    id: "0".repeat(64),
    pubkey: "f".repeat(64),
    created_at: 1_000,
    kind: KIND_STREAM_MESSAGE,
    tags: [],
    content: "",
    sig: "",
    ...overrides,
  };
}

function threadedReply(id, createdAt) {
  return relayEvent({
    id,
    created_at: createdAt,
    tags: [
      ["e", ROOT_ID, "", "root"],
      ["e", ROOT_ID, "", "reply"],
    ],
  });
}

// Stub the relay/tauri boundary: expose the live listener and count thread
// history fetches. The mount-fetch also fires a best-effort aux backfill; stub
// those to empty so the harness stays hermetic.
function stubBoundary() {
  const original = {
    subscribeToChannelLive: relayClient.subscribeToChannelLive,
    subscribeToReconnects: relayClient.subscribeToReconnects,
    setVisibleChannelId: relayClient.setVisibleChannelId,
    fetchAuxEventsByReference: relayClient.fetchAuxEventsByReference,
    fetchAuxDeletionEventsForAuxEvents:
      relayClient.fetchAuxDeletionEventsForAuxEvents,
  };
  let liveListener = () => {};
  let fetchCount = 0;
  relayClient.subscribeToChannelLive = async (_channelId, listener) => {
    liveListener = listener;
    return async () => {};
  };
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.setVisibleChannelId = () => {};
  relayClient.fetchAuxEventsByReference = async () => [];
  relayClient.fetchAuxDeletionEventsForAuxEvents = async () => [];

  const prevInvoke = globalThis.__TAURI_INTERNALS__;
  const internals = {
    invoke: async (cmd) => {
      if (cmd === "get_thread_replies") {
        fetchCount += 1;
        // Production walks `thread_metadata` on the relay, so a persisted live
        // reply is part of the server subtree. Returning both models that: the
        // fixed producer drops the pre-open live reply from cache, and the
        // history fetch is what restores it — unioned exactly once.
        return {
          events: [
            threadedReply(HISTORY_REPLY_ID, 1_500),
            threadedReply(LIVE_REPLY_ID, 2_000),
          ],
          next_cursor: null,
        };
      }
      return { events: [], next_cursor: null };
    },
    transformCallback: () => Math.random(),
  };
  globalThis.__TAURI_INTERNALS__ = internals;
  // `@tauri-apps/api/core` reads `window.__TAURI_INTERNALS__`; JSDOM's `window`
  // is a distinct object from `globalThis`, so set it on both.
  globalThis.window.__TAURI_INTERNALS__ = internals;

  return {
    emit: (event) => liveListener(event),
    fetches: () => fetchCount,
    restore() {
      Object.assign(relayClient, original);
      globalThis.__TAURI_INTERNALS__ = prevInvoke;
    },
  };
}

afterEach(() => {
  delete globalThis.window;
  delete globalThis.document;
  delete globalThis.HTMLElement;
  delete globalThis.IS_REACT_ACT_ENVIRONMENT;
});

test("a live reply to a never-opened thread does not skip the mount history fetch", async () => {
  const dom = installDom();
  const stub = stubBoundary();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.mount();

  const channel = { id: CHANNEL_ID, channelType: "channel" };
  function Harness() {
    useChannelSubscription(channel);
    return null;
  }
  const root = createRoot(dom.window.document.getElementById("root"));
  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => setImmediate(resolve));
    });
  };

  try {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Harness),
        ),
      );
    });
    // Let the async subscribe promise resolve so the live listener registers.
    await settle();

    const key = threadRepliesKey(CHANNEL_ID, ROOT_ID);
    // Precondition: the thread has never been opened, so its key is absent.
    assert.equal(
      queryClient.getQueryData(key),
      undefined,
      "precondition: never-opened thread has no cache entry",
    );

    // A live reply arrives for that never-opened thread, through the real
    // producer path.
    await act(async () => {
      stub.emit(threadedReply(LIVE_REPLY_ID, 2_000));
    });
    await settle();

    // The producer must not have created a fresh cache from the live reply.
    assert.equal(
      queryClient.getQueryData(key),
      undefined,
      "a live reply must not create a fresh cache for a never-opened thread",
    );

    // Model the thread open exactly as `useThreadReplies` does: fetchQuery with
    // the same key/staleTime. If the producer had minted a fresh key, fetchQuery
    // would return it without invoking the queryFn.
    await queryClient.fetchQuery({
      queryKey: key,
      queryFn: () => loadThreadReplies(queryClient, CHANNEL_ID, ROOT_ID),
      staleTime: THREAD_REPLIES_STALE_TIME_MS,
    });
    await settle();

    assert.equal(
      stub.fetches(),
      1,
      "opening the thread must run the history fetch, not treat a live-reply cache as fresh",
    );

    const ids = (queryClient.getQueryData(key) ?? []).map((event) => event.id);
    assert.deepEqual(
      [...ids].sort(),
      [HISTORY_REPLY_ID, LIVE_REPLY_ID].sort(),
      "the fetched subtree unions the in-flight live reply exactly once",
    );
  } finally {
    act(() => root.unmount());
    queryClient.clear();
    queryClient.unmount();
    stub.restore();
  }
});
