import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";

import { useChannelSubscription, useSendMessageMutation } from "./hooks.ts";
import {
  channelMessagesKey,
  channelWindowKey,
  threadRepliesKey,
} from "./lib/messageQueryKeys.ts";
import {
  emptyChannelWindowStore,
  flattenChannelWindowEvents,
  replaceNewestChannelWindow,
} from "./lib/channelWindowStore.ts";
import { relayClient } from "../../shared/api/relayClient.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
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
  mock.restoreAll();
});
after(() => dom.window.close());

const identity = { pubkey: "a".repeat(64) };
const channel = {
  id: "test-channel",
  name: "test",
  channelType: "stream",
  participantPubkeys: [],
};
function event(id, content, tags = []) {
  return {
    id: id.repeat(64),
    pubkey: "b".repeat(64),
    created_at: 100,
    kind: 9,
    tags: [["h", channel.id], ...tags],
    content,
    sig: "",
  };
}

async function harness() {
  const { act, renderHook, waitFor } = await import("@testing-library/react");
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const messagesKey = channelMessagesKey(channel.id);
  const windowKey = channelWindowKey(channel.id);
  const initial = event("1", "original message");
  client.setQueryData(messagesKey, [initial]);
  client.setQueryData(
    windowKey,
    replaceNewestChannelWindow(emptyChannelWindowStore(), {
      startCursor: null,
      rows: [{ event: initial, thread: null }],
      aux: [],
      nextCursor: null,
      hasMore: false,
    }),
  );
  const sends = [];
  mock.method(
    relayClient,
    "sendMessage",
    () => new Promise((resolve, reject) => sends.push({ resolve, reject })),
  );
  let receive;
  mock.method(relayClient, "subscribeToChannelLive", async (_id, callback) => {
    receive = callback;
    return async () => {};
  });
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  const view = renderHook(
    () => {
      useChannelSubscription(channel);
      return useSendMessageMutation(channel, identity);
    },
    {
      wrapper: ({ children }) =>
        React.createElement(QueryClientProvider, { client }, children),
    },
  );
  await waitFor(() => assert.ok(receive));
  return {
    client,
    sends,
    receive: (incoming) => act(() => receive(incoming)),
    async send(content) {
      const count = sends.length;
      let settled;
      await act(async () => {
        settled = view.result.current
          .mutateAsync({ content })
          .catch((error) => error);
      });
      await waitFor(() => assert.equal(sends.length, count + 1));
      return { settle: () => settled };
    },
    async reject(index, pending) {
      await act(async () => {
        sends[index].reject(
          new Error("Connection lost before acknowledgement"),
        );
        assert.match((await pending.settle()).message, /Connection lost/);
      });
    },
    contents: () => client.getQueryData(messagesKey).map((row) => row.content),
    windowContents: () =>
      flattenChannelWindowEvents(client.getQueryData(windowKey)).map(
        (row) => row.content,
      ),
  };
}

test("failed send preserves agent messages and thread replies received while pending", async () => {
  const h = await harness();
  try {
    const pending = await h.send("failed message");
    h.receive(event("2", "agent result"));
    h.receive(
      event("3", "agent thread reply", [["e", "1".repeat(64), "", "reply"]]),
    );
    assert.ok(h.contents().includes("agent result"));
    await h.reject(0, pending);
    assert.deepEqual(
      new Set(h.contents()),
      new Set(["original message", "agent result"]),
    );
    assert.deepEqual(
      h.client
        .getQueryData(threadRepliesKey(channel.id, "1".repeat(64)))
        .map((row) => row.content),
      ["agent thread reply"],
    );
    assert.deepEqual(new Set(h.windowContents()), new Set(h.contents()));
    h.receive(event("4", "next event"));
    assert.ok(
      h.contents().includes("agent result"),
      "later projection must retain the reply",
    );
  } finally {
    h.client.clear();
  }
});

test("overlapping failed sends remove only their own optimistic messages", async () => {
  const h = await harness();
  try {
    const first = await h.send("first pending");
    const second = await h.send("second pending");
    await h.reject(0, first);
    assert.deepEqual(
      new Set(h.contents()),
      new Set(["original message", "second pending"]),
    );
    await h.reject(1, second);
    assert.deepEqual(h.contents(), ["original message"]);
    assert.deepEqual(h.windowContents(), ["original message"]);
  } finally {
    h.client.clear();
  }
});
