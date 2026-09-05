import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

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

after(() => dom.window.close());

test("membership refresh survives an in-flight channel fetch", async () => {
  const React = await import("react");
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { relayClient } = await import("@/shared/api/relayClient");
  const { KIND_MEMBER_ADDED_NOTIFICATION } = await import(
    "@/shared/constants/kinds"
  );
  const { useMembershipNotifications } = await import(
    "./useMembershipNotifications.ts"
  );

  const originalSubscribeLive = relayClient.subscribeLive;
  let listener;
  relayClient.subscribeLive = async (_filter, nextListener) => {
    listener = nextListener;
    return async () => {};
  };

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const invalidations = [];
  queryClient.invalidateQueries = async ({ queryKey }) => {
    invalidations.push(queryKey);
  };
  let fetching = 1;
  queryClient.isFetching = () => fetching;
  const wrapper = ({ children }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);

  try {
    renderHook(() => useMembershipNotifications("viewer-pubkey"), { wrapper });
    await act(async () => new Promise((resolve) => setImmediate(resolve)));

    await act(async () => {
      listener({
        id: "membership",
        pubkey: "relay",
        created_at: 1,
        kind: KIND_MEMBER_ADDED_NOTIFICATION,
        tags: [
          ["p", "viewer-pubkey"],
          ["h", "new-channel"],
        ],
        content: "",
        sig: "sig",
      });
    });

    assert.deepEqual(invalidations, [
      ["channels", "new-channel", "detail"],
      ["channels", "new-channel", "members"],
    ]);

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 600));
    });
    assert.equal(
      invalidations.filter((key) => key.length === 1).length,
      0,
      "must not invalidate the channel list while get_channels is in flight",
    );

    fetching = 0;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 600));
    });
    assert.deepEqual(invalidations, [
      ["channels", "new-channel", "detail"],
      ["channels", "new-channel", "members"],
      ["channels"],
    ]);
  } finally {
    cleanup();
    queryClient.clear();
    relayClient.subscribeLive = originalSubscribeLive;
  }
});
