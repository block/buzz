/**
 * Seam test for the warm-cache producer contract that makes the thread-replies
 * `staleTime: 30s` safe.
 *
 * A warm thread-replies cache is only trustworthy on reopen because the active
 * channel's live subscription keeps it current: `useChannelSubscription`'s
 * `appendMessage` writes each live event into the cache. Two producer paths
 * carry that contract, and their key scoping is load-bearing:
 *
 *   - A live threaded *content* event writes only to its own
 *     `(channel, root)` thread-replies key -- a reply under one root must not
 *     leak into a sibling thread's cache.
 *   - A live *aux* event (edit/reaction/deletion) carries no thread reference,
 *     so it fans out across every existing `["thread-replies", channelId]`
 *     cache -- an overlay can apply to any thread the reaction targets.
 *
 * The existing `useChannelSubscription.test.mjs` pins invalidation and this
 * file pins the producers. Falsifying either key scoping (content fan-out, or
 * aux narrowing to one key) fails the matching assertion.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import {
  KIND_REACTION,
  KIND_STREAM_MESSAGE,
} from "@/shared/constants/kinds.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import { threadRepliesKey } from "./lib/messageQueryKeys.ts";
import { useChannelSubscription } from "./hooks.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const ROOT_A = "a".repeat(64);
const ROOT_B = "b".repeat(64);

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

// Replace the relay singleton's network methods with resolved stubs and expose
// the live-event listener so the test can push events through the real
// `appendMessage` producer path.
function stubRelayClient() {
  const original = {
    subscribeToChannelLive: relayClient.subscribeToChannelLive,
    subscribeToReconnects: relayClient.subscribeToReconnects,
    setVisibleChannelId: relayClient.setVisibleChannelId,
  };
  let liveListener = () => {};
  relayClient.subscribeToChannelLive = async (_channelId, listener) => {
    liveListener = listener;
    return async () => {};
  };
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.setVisibleChannelId = () => {};
  return {
    emit: (event) => liveListener(event),
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

async function mountSubscription() {
  const dom = installDom();
  const stub = stubRelayClient();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.mount();

  const keyA = threadRepliesKey(CHANNEL_ID, ROOT_A);
  const keyB = threadRepliesKey(CHANNEL_ID, ROOT_B);
  // Two warm sibling-thread caches so a leak across roots is observable.
  queryClient.setQueryData(keyA, [relayEvent({ id: ROOT_A })]);
  queryClient.setQueryData(keyB, [relayEvent({ id: ROOT_B })]);

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
  // Let the async subscribe promise resolve so the live listener is registered.
  await settle();

  const idsIn = (key) =>
    (queryClient.getQueryData(key) ?? []).map((event) => event.id);

  return {
    idsIn,
    keyA,
    keyB,
    async emit(event) {
      await act(async () => {
        stub.emit(event);
      });
    },
    cleanup() {
      act(() => root.unmount());
      queryClient.unmount();
      stub.restore();
    },
  };
}

test("a live threaded content event updates only its own (channel, root) key", async () => {
  const h = await mountSubscription();
  try {
    const replyId = "1".repeat(64);
    await h.emit(
      relayEvent({
        id: replyId,
        kind: KIND_STREAM_MESSAGE,
        created_at: 2_000,
        tags: [
          ["e", ROOT_A, "", "root"],
          ["e", ROOT_A, "", "reply"],
        ],
      }),
    );
    assert.deepEqual(
      h.idsIn(h.keyA),
      [ROOT_A, replyId],
      "the reply must append to its own root's thread-replies cache",
    );
    assert.deepEqual(
      h.idsIn(h.keyB),
      [ROOT_B],
      "the reply must not leak into a sibling thread's cache",
    );
  } finally {
    h.cleanup();
  }
});

test("a live aux event updates every existing thread key in the channel", async () => {
  const h = await mountSubscription();
  try {
    const reactionId = "2".repeat(64);
    await h.emit(
      relayEvent({
        id: reactionId,
        kind: KIND_REACTION,
        created_at: 2_000,
        content: "+",
        tags: [["e", ROOT_A, "", "reply"]],
      }),
    );
    assert.deepEqual(
      h.idsIn(h.keyA),
      [ROOT_A, reactionId],
      "an aux overlay must fan out to the channel's existing thread caches",
    );
    assert.deepEqual(
      h.idsIn(h.keyB),
      [ROOT_B, reactionId],
      "an aux overlay must fan out to the channel's existing thread caches",
    );
  } finally {
    h.cleanup();
  }
});
