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

test("DM visibility updates refresh the channel list without treating snapshots as membership changes", async () => {
  const React = await import("react");
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { relayClient } = await import("@/shared/api/relayClient");
  const {
    KIND_DM_VISIBILITY,
    KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION,
  } = await import("@/shared/constants/kinds");
  const { useMembershipNotifications } = await import(
    "./useMembershipNotifications.ts"
  );

  const originalSubscribeLive = relayClient.subscribeLive;
  let subscriptionFilter;
  let onEvent;
  relayClient.subscribeLive = async (filter, listener) => {
    subscriptionFilter = filter;
    onEvent = listener;
    return async () => {};
  };

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const invalidations = [];
  queryClient.invalidateQueries = async ({ queryKey }) => {
    invalidations.push(queryKey);
  };
  const wrapper = ({ children }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);

  try {
    const { unmount } = renderHook(
      () => useMembershipNotifications("Viewer-Pubkey"),
      { wrapper },
    );
    await act(async () => new Promise((resolve) => setImmediate(resolve)));

    assert.deepEqual(subscriptionFilter.kinds, [
      KIND_DM_VISIBILITY,
      KIND_MEMBER_ADDED_NOTIFICATION,
      KIND_MEMBER_REMOVED_NOTIFICATION,
    ]);
    assert.deepEqual(subscriptionFilter["#p"], ["viewer-pubkey"]);

    await act(async () => {
      onEvent({
        id: "visibility",
        pubkey: "relay",
        created_at: 1,
        kind: KIND_DM_VISIBILITY,
        tags: [
          ["p", "viewer-pubkey"],
          ["h", "still-hidden-dm"],
        ],
        content: "",
        sig: "sig",
      });
    });
    assert.deepEqual(invalidations, [["channels"]]);

    invalidations.length = 0;
    await act(async () => {
      onEvent({
        id: "membership",
        pubkey: "relay",
        created_at: 2,
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
      ["channels"],
      ["channels", "new-channel", "detail"],
      ["channels", "new-channel", "members"],
    ]);

    unmount();
  } finally {
    cleanup();
    queryClient.clear();
    relayClient.subscribeLive = originalSubscribeLive;
  }
});
