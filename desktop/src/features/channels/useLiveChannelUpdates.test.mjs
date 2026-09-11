import assert from "node:assert/strict";
import { after, afterEach, mock, test } from "node:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";

import { useLiveChannelUpdates } from "./useLiveChannelUpdates.ts";
import {
  channelMessagesKey,
  threadRepliesKey,
} from "@/features/messages/lib/messageQueryKeys.ts";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages.ts";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  mock.restoreAll();
});

after(() => dom.window.close());

const CHANNEL_ID = "channel-a";
const ROOT_ID = "a".repeat(64);
const PARENT_ID = "1".repeat(64);
const EXISTING_REPLY_ID = "b".repeat(64);
const NEW_REPLY_ID = "c".repeat(64);
const TOP_LEVEL_ID = "d".repeat(64);
const BROADCAST_REPLY_ID = "e".repeat(64);

const channel = {
  id: CHANNEL_ID,
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

function event(id, createdAt, tags, content = id) {
  return {
    id,
    pubkey: "f".repeat(64),
    created_at: createdAt,
    kind: KIND_STREAM_MESSAGE,
    tags,
    content,
    sig: "0".repeat(128),
  };
}

function rootEvent() {
  return event(ROOT_ID, 100, [["h", CHANNEL_ID]], "root");
}

function directReplyEvent(id, createdAt, extraTags = [], content = id) {
  return event(
    id,
    createdAt,
    [["h", CHANNEL_ID], ["e", ROOT_ID, "", "reply"], ...extraTags],
    content,
  );
}

async function mountLiveUpdates(client) {
  let callback = null;
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "subscribeLive", async (_filter, onEvent) => {
    callback = onEvent;
    return async () => {};
  });

  const { renderHook, waitFor } = await import("@testing-library/react");
  const view = renderHook(() => useLiveChannelUpdates([channel], null), {
    wrapper: ({ children }) =>
      React.createElement(QueryClientProvider, { client }, children),
  });

  await waitFor(() => assert.equal(typeof callback, "function"));

  return {
    callback,
    unmount: () => view.unmount(),
  };
}

test("non-broadcast replies feed the raw projection and root thread cache without rendering as timeline rows", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);
  const existingReply = directReplyEvent(
    EXISTING_REPLY_ID,
    101,
    [],
    "existing-thread-reply",
  );
  client.setQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID), [existingReply]);

  const { callback, unmount } = await mountLiveUpdates(client);
  const reply = directReplyEvent(NEW_REPLY_ID, 102, [], "new-thread-reply");

  callback(reply);

  const rawMessages = client.getQueryData(channelMessagesKey(CHANNEL_ID));
  assert.deepEqual(rawMessages, [rootEvent(), reply]);
  assert.deepEqual(client.getQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID)), [
    existingReply,
    reply,
  ]);
  const entries = buildMainTimelineEntries(
    formatTimelineMessages(rawMessages, channel, undefined, null),
  );
  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.message.id,
      summaryThreadHeadId: entry.summary?.threadHeadId ?? null,
      summaryReplyCount: entry.summary?.replyCount ?? 0,
    })),
    [
      {
        id: ROOT_ID,
        summaryThreadHeadId: ROOT_ID,
        summaryReplyCount: 1,
      },
    ],
  );

  unmount();
  client.clear();
});

test("nested non-broadcast replies use the root thread cache instead of the parent cache", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);
  const parentReply = directReplyEvent(PARENT_ID, 101, [], "parent-reply");
  client.setQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID), [parentReply]);

  const { callback, unmount } = await mountLiveUpdates(client);
  const nestedReply = event(
    NEW_REPLY_ID,
    102,
    [
      ["h", CHANNEL_ID],
      ["e", ROOT_ID, "", "root"],
      ["e", PARENT_ID, "", "reply"],
    ],
    "nested-reply",
  );

  callback(nestedReply);

  assert.deepEqual(client.getQueryData(channelMessagesKey(CHANNEL_ID)), [
    rootEvent(),
    nestedReply,
  ]);
  assert.deepEqual(client.getQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID)), [
    parentReply,
    nestedReply,
  ]);
  assert.equal(
    client.getQueryCache().find({
      queryKey: threadRepliesKey(CHANNEL_ID, PARENT_ID),
      exact: true,
    }),
    undefined,
  );

  unmount();
  client.clear();
});

test("non-broadcast replies merge into an existing empty thread query", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);
  client.getQueryCache().build(client, {
    queryKey: threadRepliesKey(CHANNEL_ID, ROOT_ID),
  });

  const { callback, unmount } = await mountLiveUpdates(client);
  const reply = directReplyEvent(NEW_REPLY_ID, 102, [], "new-thread-reply");

  callback(reply);

  assert.deepEqual(client.getQueryData(channelMessagesKey(CHANNEL_ID)), [
    rootEvent(),
    reply,
  ]);
  assert.deepEqual(client.getQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID)), [
    reply,
  ]);

  unmount();
  client.clear();
});

test("non-broadcast replies do not create thread cache entries for unopened threads", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);

  const { callback, unmount } = await mountLiveUpdates(client);
  const reply = directReplyEvent(NEW_REPLY_ID, 102, [], "new-thread-reply");

  callback(reply);

  assert.deepEqual(client.getQueryData(channelMessagesKey(CHANNEL_ID)), [
    rootEvent(),
    reply,
  ]);
  assert.equal(
    client.getQueryCache().find({
      queryKey: threadRepliesKey(CHANNEL_ID, ROOT_ID),
      exact: true,
    }),
    undefined,
  );

  unmount();
  client.clear();
});

test("top-level live messages still merge into channelMessagesKey", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);

  const { callback, unmount } = await mountLiveUpdates(client);
  const topLevel = event(
    TOP_LEVEL_ID,
    103,
    [["h", CHANNEL_ID]],
    "top-level-message",
  );

  callback(topLevel);

  assert.deepEqual(client.getQueryData(channelMessagesKey(CHANNEL_ID)), [
    rootEvent(),
    topLevel,
  ]);

  unmount();
  client.clear();
});

test("broadcast replies keep the top-level channel write and update an existing thread cache", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(channelMessagesKey(CHANNEL_ID), [rootEvent()]);
  const existingReply = directReplyEvent(
    EXISTING_REPLY_ID,
    101,
    [],
    "existing-thread-reply",
  );
  client.setQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID), [existingReply]);

  const { callback, unmount } = await mountLiveUpdates(client);
  const broadcastReply = directReplyEvent(
    BROADCAST_REPLY_ID,
    104,
    [["broadcast", "1"]],
    "broadcast-reply",
  );

  callback(broadcastReply);

  assert.deepEqual(client.getQueryData(channelMessagesKey(CHANNEL_ID)), [
    rootEvent(),
    broadcastReply,
  ]);
  assert.deepEqual(client.getQueryData(threadRepliesKey(CHANNEL_ID, ROOT_ID)), [
    existingReply,
    broadcastReply,
  ]);

  unmount();
  client.clear();
});
