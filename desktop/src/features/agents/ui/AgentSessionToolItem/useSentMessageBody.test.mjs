import assert from "node:assert/strict";
import test from "node:test";

import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  resolveSentMessageBody,
  sentMessageBodyQueryOptions,
  shouldFetchSentMessage,
  useSentMessageBody,
} from "./useSentMessageBody.ts";

const link = { channelId: "ch-1", messageId: "ev-1" };

test("shouldFetchSentMessage returns true when messageLink present and no inline content", () => {
  assert.equal(shouldFetchSentMessage(link, null), true);
});

test("shouldFetchSentMessage returns false when inline content is present", () => {
  assert.equal(shouldFetchSentMessage(link, "hello"), false);
});

test("shouldFetchSentMessage returns false when messageLink is null", () => {
  assert.equal(shouldFetchSentMessage(null, null), false);
});

test("resolveSentMessageBody returns inline content over fetched content", () => {
  assert.equal(
    resolveSentMessageBody("inline text", "fetched text"),
    "inline text",
  );
});

test("resolveSentMessageBody returns fetched content when no inline content", () => {
  assert.equal(resolveSentMessageBody(null, "fetched text"), "fetched text");
});

test("resolveSentMessageBody returns null when both inline and fetched are absent", () => {
  assert.equal(resolveSentMessageBody(null, undefined), null);
  assert.equal(resolveSentMessageBody(null, null), null);
});

// ── Integration: the real useQuery wiring, not just the pure helpers ──────────
//
// `sentMessageBodyQueryOptions` is the exact options object `useSentMessageBody`
// hands to `useQuery`. Driving it through query-core's `QueryObserver` (what
// `useQuery` constructs under the hood) exercises the production fetch/cache
// wiring without a DOM — these tests fail if the hook stops calling the
// injected fetcher, stops keying by `messageId`, or stops gating on
// `enabled`.

test("useSentMessageBody wiring fetches by messageId and resolves fetched content", async () => {
  const queryClient = new QueryClient();
  let calledWith = null;
  const fetchEventById = async (eventId) => {
    calledWith = eventId;
    return { content: "hello from the event" };
  };
  const options = sentMessageBodyQueryOptions(link, null, fetchEventById);
  const observer = new QueryObserver(queryClient, options);
  const unsubscribe = observer.subscribe(() => {});
  await new Promise((resolve) => setTimeout(resolve, 20));
  unsubscribe();

  assert.equal(calledWith, "ev-1");
  assert.equal(
    resolveSentMessageBody(null, observer.getCurrentResult().data?.content),
    "hello from the event",
  );
});

test("useSentMessageBody wiring never invokes the fetcher when inline content is present", async () => {
  const queryClient = new QueryClient();
  let called = false;
  const fetchEventById = async () => {
    called = true;
    return { content: "unused" };
  };
  const options = sentMessageBodyQueryOptions(
    link,
    "inline text",
    fetchEventById,
  );
  const observer = new QueryObserver(queryClient, options);
  const unsubscribe = observer.subscribe(() => {});
  await new Promise((resolve) => setTimeout(resolve, 20));
  unsubscribe();

  assert.equal(called, false);
});

test("useSentMessageBody wiring leaves data undefined on a rejected fetch (fetch-miss path)", async () => {
  const queryClient = new QueryClient();
  const fetchEventById = async () => {
    throw new Error("relay miss");
  };
  const options = sentMessageBodyQueryOptions(link, null, fetchEventById);
  const observer = new QueryObserver(queryClient, { ...options, retry: false });
  const unsubscribe = observer.subscribe(() => {});
  await new Promise((resolve) => setTimeout(resolve, 20));
  unsubscribe();

  assert.equal(observer.getCurrentResult().data, undefined);
});

// ── Render-level coverage ──────────────────────────────────────────────────
//
// Renders the hook through a real QueryClientProvider. Cache is pre-seeded
// via `setQueryData` (equivalent to a settled fetch) for the render
// assertion, since `renderToStaticMarkup` never commits and so never runs
// the query's mount effect — the wiring tests above already prove the
// fetcher is called through the real `useQuery` path.

function SentMessageBodyProbe({ messageLink, inlineContent, fetchEventById }) {
  const content = useSentMessageBody(
    messageLink,
    inlineContent,
    fetchEventById,
  );
  return React.createElement(
    "span",
    null,
    content ?? "Message content unavailable.",
  );
}

function renderProbe(queryClient, props) {
  return renderToStaticMarkup(
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(SentMessageBodyProbe, props),
    ),
  );
}

test("CompactMessageSummary renders fetched event content when a cached fetch has resolved", () => {
  const queryClient = new QueryClient();
  queryClient.setQueryData(["sent-message-body", link.messageId], {
    content: "hello from the event",
  });
  const fetchEventById = async () => {
    throw new Error("should not be called — cache hit");
  };

  const html = renderProbe(queryClient, {
    messageLink: link,
    inlineContent: null,
    fetchEventById,
  });

  assert.match(html, /hello from the event/);
});

test("CompactMessageSummary renders the placeholder, never a raw flag value, on a fetch miss", () => {
  const queryClient = new QueryClient();
  const fetchEventById = async () => {
    throw new Error("relay miss");
  };

  const html = renderProbe(queryClient, {
    messageLink: link,
    inlineContent: null,
    fetchEventById,
  });

  assert.match(html, /Message content unavailable\./);
});
