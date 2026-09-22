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
    localStorage: dom.window.localStorage,
  });
});
after(() => dom.window.close());

const rawChannels = ["a", "b"].map((id) => ({
  id,
  name: id,
  channel_type: "stream",
  visibility: "private",
  description: "",
  member_count: 1,
  member_pubkeys: ["viewer"],
  archived_at: null,
  last_message_at: null,
  participants: [],
  participant_pubkeys: [],
  is_member: true,
}));
const revoked = "restricted: channel access revoked";

async function mount() {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const React = await import("react");
  const { QueryClient, QueryClientProvider, useQuery } = await import(
    "@tanstack/react-query"
  );
  const { relayClient } = await import("@/shared/api/relayClient");
  const { fromRawChannel } = await import("@/shared/api/tauriChannels");
  const { channelsQueryKey, refreshChannelsQuery } = await import("./hooks.ts");
  const { useLiveChannelUpdates } = await import("./useLiveChannelUpdates.ts");
  const original = {
    setTimeout: window.setTimeout,
    clearTimeout: window.clearTimeout,
    internals: window.__TAURI_INTERNALS__,
  };
  const timers = new Map();
  let now = 0;
  let nextTimer = 0;
  window.setTimeout = (fn, ms) => {
    timers.set(++nextTimer, { fn, at: now + ms });
    return nextTimer;
  };
  window.clearTimeout = (id) => timers.delete(id);
  const sent = [];
  const reads = [];
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command === "plugin:websocket|send") {
        const frame = JSON.parse(args.message.data);
        sent.push(frame);
        if (frame[0] === "REQ")
          queueMicrotask(() => deliver(["EOSE", frame[1]]));
        return;
      }
      if (command === "get_channels") {
        return new Promise((resolve, reject) =>
          reads.push({ args, resolve, reject }),
        );
      }
      assert.fail(`Unexpected IPC command: ${command}`);
    },
  };
  // Stub only native transport, not subscription creation or CLOSED dispatch.
  relayClient.wsId = 7;
  const deliver = (frame, generation = relayClient.connectionGeneration) =>
    relayClient.handleWsMessage(JSON.stringify(frame), generation);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  const hook = renderHook(
    () => {
      const query = useQuery({
        queryKey: channelsQueryKey,
        queryFn: () =>
          refreshChannelsQuery({
            queryClient,
            initialSnapshotPair: null,
            relayUrl: null,
            ownerPubkey: null,
          }),
        initialData: rawChannels.map(fromRawChannel),
        staleTime: Infinity,
      });
      useLiveChannelUpdates(query.data, null);
      return query;
    },
    {
      wrapper: ({ children }) =>
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          children,
        ),
    },
  );
  const settle = () =>
    act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  await settle();
  assert.equal(sent.filter(([type]) => type === "REQ").length, 2);
  const ids = sent.filter(([type]) => type === "REQ").map(([, id]) => id);
  return {
    reads,
    sent,
    ids,
    timers,
    relayClient,
    queryClient,
    channelsQueryKey,
    state: () => queryClient.getQueryState(channelsQueryKey),
    data: () => queryClient.getQueryData(channelsQueryKey),
    close: (id = ids[0], reason = revoked, generation) =>
      act(async () => {
        await deliver(["CLOSED", id, reason], generation);
      }),
    async tick(ms) {
      await act(async () => {
        now += ms;
        for (const [id, timer] of [...timers]) {
          if (timer.at <= now) {
            timers.delete(id);
            timer.fn();
          }
        }
      });
      await settle();
    },
    async respond(channels = rawChannels) {
      assert.equal(
        reads.length,
        1,
        "one authoritative refresh must be pending",
      );
      await act(async () =>
        reads[0].resolve({ hash: "fresh", channels, last_messages: {} }),
      );
      await settle();
    },
    async fail() {
      assert.equal(
        reads.length,
        1,
        "one authoritative refresh must be pending",
      );
      await act(async () => reads[0].reject(new Error("fixture offline")));
      await settle();
    },
    unmount: hook.unmount,
    restore() {
      hook.unmount();
      cleanup();
      queryClient.clear();
      // Avoid a native disconnect command; clear the real session's listeners/state.
      relayClient.wsId = null;
      relayClient.disconnect();
      window.setTimeout = original.setTimeout;
      window.clearTimeout = original.clearTimeout;
      window.__TAURI_INTERNALS__ = original.internals;
    },
  };
}

for (const outcome of ["archived", "unchanged", "removed", "failed"]) {
  test(`CLOSED refreshes authoritative channel state: ${outcome}`, async () => {
    const h = await mount();
    try {
      await h.close();
      await h.close(); // duplicate frame is not a second hint
      await h.close(h.ids[1]); // a second channel shares the debounce
      assert.equal(
        h.relayClient.subscriptions.size,
        0,
        "terminal subscriptions stay removed",
      );
      assert.equal(h.reads.length, 0);
      assert.ok(
        h
          .data()
          .every((channel) => channel.archivedAt === null && channel.isMember),
      );
      await h.tick(499);
      assert.equal(h.reads.length, 0);
      await h.tick(1);
      assert.equal(
        h.reads.length,
        1,
        "burst coalesces into one query without waiting for polling",
      );
      assert.equal(h.data().length, 2, "CLOSED alone cannot remove membership");
      if (outcome === "failed") {
        await h.fail();
        assert.equal(h.state().status, "error");
        assert.equal(h.data().length, 2);
        assert.ok(
          h
            .data()
            .every(
              (channel) => channel.archivedAt === null && channel.isMember,
            ),
        );
      } else {
        await h.respond(
          outcome === "archived"
            ? [
                { ...rawChannels[0], archived_at: "2026-09-21T15:36:31Z" },
                rawChannels[1],
              ]
            : outcome === "removed"
              ? rawChannels.slice(1)
              : rawChannels,
        );
        assert.deepEqual(
          h.data().map((channel) => channel.id),
          outcome === "removed" ? ["b"] : ["a", "b"],
        );
        assert.equal(
          h.data().find((channel) => channel.id === "a")?.archivedAt,
          outcome === "archived"
            ? "2026-09-21T15:36:31Z"
            : outcome === "removed"
              ? undefined
              : null,
        );
      }
      await h.tick(30_000);
      assert.equal(
        h.reads.length,
        1,
        "refresh failure or unchanged state must not loop",
      );
      assert.equal(
        h.sent.filter(([type]) => type === "REQ").length,
        2,
        "no terminal resubscribe",
      );
    } finally {
      h.restore();
    }
  });
}

test("unknown, non-channel, unrelated and old-generation CLOSED do not refresh channels", async () => {
  const h = await mount();
  try {
    await h.close("unknown");
    await h.close(h.ids[0], revoked, h.relayClient.connectionGeneration - 1);
    await h.close(h.ids[0], "invalid: bad filter");
    const dispose = await h.relayClient.subscribeLive(
      { kinds: [0], limit: 1 },
      () => {},
    );
    const globalId = h.sent.at(-1)[1];
    await h.close(globalId);
    await dispose();
    await h.tick(500);
    assert.equal(h.reads.length, 0);
    assert.equal(h.relayClient.subscriptions.size, 1);
  } finally {
    h.restore();
  }
});

test("a different restricted CLOSED reason does not refresh channels", async () => {
  const h = await mount();
  try {
    await h.close(h.ids[0], "restricted: not a channel member");
    assert.equal(
      h.relayClient.subscriptions.has(h.ids[0]),
      false,
      "the known channel subscription is still terminal",
    );
    await h.tick(500);
    assert.equal(
      h.reads.length,
      0,
      "only the exact revocation reason refreshes",
    );
  } finally {
    h.restore();
  }
});

test("unmount cancels queued invalidation and removes its session listener", async () => {
  const h = await mount();
  try {
    await h.close();
    h.unmount();
    await h.tick(500);
    assert.equal(h.reads.length, 0);
    const dispose = await h.relayClient.subscribeLive(
      { kinds: [9], "#h": ["later"], limit: 1 },
      () => {},
    );
    await h.close(h.sent.at(-1)[1]);
    await h.tick(500);
    assert.equal(
      h.timers.size,
      0,
      "unmounted listener must not schedule another invalidation",
    );
    await dispose();
  } finally {
    h.restore();
  }
});
