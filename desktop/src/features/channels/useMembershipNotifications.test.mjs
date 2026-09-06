/**
 * Tests for the membership-notification channel-list refresh gate.
 *
 * These run against a real QueryClient and drive the 500ms trailing debounce
 * with node:test fake timers, so there are no wall-clock sleeps. `isFetching`
 * is deliberately never stubbed: the behaviour under test is which queries the
 * gate's filter matches, and a wholesale stub would answer that question for
 * the seam instead of exercising it.
 */

import assert from "node:assert/strict";
import { after, before, mock, test } from "node:test";

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

const CHANNEL_ID = "new-channel";
const VIEWER = "viewer-pubkey";
/** Mirrors CHANNELS_INVALIDATE_DEBOUNCE_MS in useMembershipNotifications.ts. */
const DEBOUNCE_MS = 500;

/**
 * Park a real query in `fetching` until `settle()` is called. The fetch promise
 * is kept so the test can await it before teardown: a query still in flight
 * when the client is cleared leaves that promise pending forever.
 */
function startDeferredQuery(queryClient, queryKey) {
  let release;
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  const fetched = queryClient
    .fetchQuery({ queryKey, queryFn: () => pending })
    .catch(() => {});
  return {
    settle: async (value = []) => {
      release(value);
      await fetched;
    },
  };
}

async function mountHook() {
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
    // An infinite gcTime schedules no collection timer at all. A finite one is
    // armed on the mocked clock when a query settles and is still owed when the
    // clock is restored, which leaves the test runner waiting on it.
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
    },
  });
  // Wrap rather than replace, so real invalidation still runs while the keys
  // are recorded.
  const invalidated = [];
  const realInvalidateQueries = queryClient.invalidateQueries.bind(queryClient);
  queryClient.invalidateQueries = (filters, options) => {
    invalidated.push(filters?.queryKey);
    return realInvalidateQueries(filters, options);
  };

  const wrapper = ({ children }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);

  renderHook(() => useMembershipNotifications(VIEWER), { wrapper });
  await act(async () => new Promise((resolve) => setImmediate(resolve)));

  return {
    queryClient,
    invalidated,
    /** Only the bare ["channels"] list key; detail/members are 3 segments. */
    listRefreshes: () => invalidated.filter((key) => key.length === 1),
    emitMembershipEvent: async () => {
      await act(async () => {
        listener({
          id: "membership",
          pubkey: "relay",
          created_at: 1,
          kind: KIND_MEMBER_ADDED_NOTIFICATION,
          tags: [
            ["p", VIEWER],
            ["h", CHANNEL_ID],
          ],
          content: "",
          sig: "sig",
        });
      });
    },
    flush: async () => {
      await act(async () => new Promise((resolve) => setImmediate(resolve)));
    },
    dispose: () => {
      cleanup();
      queryClient.clear();
      relayClient.subscribeLive = originalSubscribeLive;
    },
  };
}

test("an in-flight members fetch does not hold back the channel list refresh", async () => {
  const harness = await mountHook();
  const { queryClient } = harness;
  try {
    // The list query is present and idle; only a sibling members fetch is in
    // flight — the exact shape this handler creates when it kicks detail and
    // members before the debounce fires.
    queryClient.setQueryData(["channels"], [{ id: "seed" }]);
    const members = startDeferredQuery(queryClient, [
      "channels",
      CHANNEL_ID,
      "members",
    ]);

    // Fixture sanity: the prefix and exact filters must genuinely disagree
    // here, or this test would pass whichever one the gate uses.
    assert.equal(
      queryClient.isFetching({ queryKey: ["channels"] }),
      1,
      "fixture: a prefix match counts the in-flight members fetch",
    );
    assert.equal(
      queryClient.isFetching({ queryKey: ["channels"], exact: true }),
      0,
      "fixture: an exact match sees the list query itself as idle",
    );

    mock.timers.enable({ apis: ["setTimeout"] });
    await harness.emitMembershipEvent();
    assert.deepEqual(
      harness.listRefreshes(),
      [],
      "the list refresh is debounced, never immediate",
    );

    mock.timers.tick(DEBOUNCE_MS);
    assert.deepEqual(
      harness.listRefreshes(),
      [["channels"]],
      "the list refresh lands even though a members fetch is still in flight",
    );

    // The members fetch really was still in flight when the refresh landed.
    assert.equal(
      queryClient.isFetching({ queryKey: ["channels"] }),
      1,
      "the members fetch outlived the list refresh it must not have blocked",
    );
    await members.settle();
  } finally {
    mock.timers.reset();
    harness.dispose();
  }
});

test("membership refresh survives an in-flight channel fetch", async () => {
  const harness = await mountHook();
  const { queryClient } = harness;
  try {
    const list = startDeferredQuery(queryClient, ["channels"]);
    assert.equal(
      queryClient.isFetching({ queryKey: ["channels"], exact: true }),
      1,
      "fixture: get_channels itself is in flight",
    );

    mock.timers.enable({ apis: ["setTimeout"] });
    await harness.emitMembershipEvent();
    assert.deepEqual(
      harness.invalidated,
      [
        ["channels", CHANNEL_ID, "detail"],
        ["channels", CHANNEL_ID, "members"],
      ],
      "detail and members refresh immediately",
    );

    mock.timers.tick(DEBOUNCE_MS);
    assert.deepEqual(
      harness.listRefreshes(),
      [],
      "must not invalidate the channel list while get_channels is in flight",
    );

    // Still parked several debounce windows later: the gate re-arms rather
    // than dropping the dirty signal.
    mock.timers.tick(DEBOUNCE_MS * 4);
    assert.deepEqual(
      harness.listRefreshes(),
      [],
      "the deferred refresh stays armed instead of firing early",
    );

    // get_channels settles; the next quiet window lets the refresh land.
    await list.settle();
    await harness.flush();
    assert.equal(
      queryClient.isFetching({ queryKey: ["channels"], exact: true }),
      0,
      "fixture: the list query really did go idle",
    );

    mock.timers.tick(DEBOUNCE_MS);
    assert.deepEqual(
      harness.invalidated,
      [
        ["channels", CHANNEL_ID, "detail"],
        ["channels", CHANNEL_ID, "members"],
        ["channels"],
      ],
      "the deferred list refresh lands once the fetch is idle",
    );
  } finally {
    mock.timers.reset();
    harness.dispose();
  }
});
