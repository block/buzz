/**
 * Mounted-hook eviction fence for the INITIAL-FETCH writer of
 * `useChannelMessagesQuery` (its `reconcileFetchedChannelWindow` queryFn).
 *
 * Unlike the deferred mutation/effect writers (fenced by the generation guard
 * in deferredWriterEviction.test.mjs), the initial fetch is the query's OWN
 * fetch, so it lives inside React Query's abort scope: when the last observer
 * unmounts or the channel switches while the fetch is in flight, React Query
 * aborts the query signal. `reconcileFetchedChannelWindow` calls
 * `signal.throwIfAborted()` BEFORE its bare `setQueryData(windowKey)` write, so
 * a fetch that settles after its unit was evicted throws instead of committing
 * — neither `channel-window` nor `channel-messages` is resurrected.
 *
 * These tests pin that contract from the mounted production hook (not a
 * projection helper) across BOTH orderings Paul named — the pre-success write
 * landing before the settle sweep, and the sweep landing first — and keep a
 * case proving the same guard still lets a normal (un-aborted) fetch populate
 * the timeline.
 *
 * DOM/IPC harness mirrors deferredWriterEviction.test.mjs.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { installDOMShim } from "@/features/channels/observedUnreadTestHarness.mjs";

installDOMShim();

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();
globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd, args) => {
    const handler = ipcHandlers.get(cmd);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback: () => Math.random(),
};

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import {
  channelMessagesKey,
  channelWindowKey,
} from "./lib/messageQueryKeys.ts";
import { emptyChannelWindowStore } from "./lib/channelWindowStore.ts";
import {
  enforceMessageWindowBounds,
  MAX_RETAINED_MESSAGE_CHANNELS,
} from "./lib/boundMessageWindows.ts";
import { resetMessageUnitGenerations } from "./lib/messageUnitGuard.ts";
import { useBoundedMessageWindows } from "./useBoundedMessageWindows.ts";
import { useChannelMessagesQuery } from "./hooks.ts";

const CAP = MAX_RETAINED_MESSAGE_CHANNELS;
const EVICTED = "evicted-channel";

/** A valid single-page head-window `/query` response with no timeline rows. */
function windowResponse(channelId) {
  return [
    {
      id: "f".repeat(64),
      pubkey: "a".repeat(64),
      created_at: 1,
      kind: 39006,
      tags: [["d", `${channelId.toLowerCase()}:head`]],
      content: JSON.stringify({ has_more: false, next_cursor: null }),
      sig: "b".repeat(128),
    },
  ];
}

/** Seed CAP fresher channels so the target channel is over cap once unpinned. */
function seedKeepsOverCap(client) {
  for (let i = 0; i < CAP; i += 1) {
    client.setQueryData(
      channelWindowKey(`keep-${i}`),
      emptyChannelWindowStore(),
      {
        updatedAt: i + 100,
      },
    );
    client.setQueryData(channelMessagesKey(`keep-${i}`), [], {
      updatedAt: i + 100,
    });
  }
}

function present(client, key) {
  return client.getQueryCache().find({ queryKey: key }) !== undefined;
}

function mountChannel(client, channel) {
  function Harness() {
    useBoundedMessageWindows(client);
    useChannelMessagesQuery(channel);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  return { root, Harness };
}

describe("initial-fetch writer — eviction fence", () => {
  beforeEach(() => {
    resetMessageUnitGenerations();
    ipcHandlers.clear();
  });

  for (const order of ["pre-success-write-then-sweep", "sweep-then-resolve"]) {
    it(`test_initial_fetch_${order.replace(/-/g, "_")}_leaves_both_keys_absent`, async () => {
      let release;
      ipcHandlers.set(
        "get_channel_window",
        (args) =>
          new Promise((resolve) => {
            release = () => resolve(windowResponse(args.channelId));
          }),
      );

      const client = new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: Infinity } },
      });
      seedKeepsOverCap(client);

      const channel = { id: EVICTED, channelType: "channel" };
      const { root, Harness } = mountChannel(client, channel);
      await act(async () => {
        root.render(
          React.createElement(
            QueryClientProvider,
            { client },
            React.createElement(Harness),
          ),
        );
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 5));
      });

      // Unmount while the fetch is in flight: the query loses its observer, so
      // React Query aborts its signal and the unit becomes evictable. (Unmount
      // outside act(): the in-flight fetch promise stays pending, so awaiting an
      // act() around the unmount would deadlock on it.)
      root.unmount();
      await new Promise((r) => setTimeout(r, 3));

      if (order === "pre-success-write-then-sweep") {
        // The fetch settles first; its pre-success window write must be
        // suppressed by throwIfAborted so the settle sweep has nothing to spare.
        await act(async () => {
          release();
          await new Promise((r) => setTimeout(r, 10));
        });
        enforceMessageWindowBounds(client);
      } else {
        // The sweep evicts first; the late fetch must not resurrect either key.
        enforceMessageWindowBounds(client);
        await act(async () => {
          release();
          await new Promise((r) => setTimeout(r, 10));
        });
      }

      await new Promise((r) => setTimeout(r, 5));

      assert.equal(
        present(client, channelWindowKey(EVICTED)),
        false,
        "initial fetch must not resurrect the window key",
      );
      assert.equal(
        present(client, channelMessagesKey(EVICTED)),
        false,
        "initial fetch must not resurrect the messages key",
      );

      // Release query-core's gc timer for any surviving cache entry so the node
      // test process exits without --test-force-exit (production `pnpm test`
      // does not force-exit; a lingering gcTime:Infinity timer would hang CI).
      client.clear();
    });
  }

  it("test_uninterrupted_initial_fetch_populates_the_timeline", async () => {
    // The complement: a fetch whose observer stays mounted is never aborted, so
    // the same guard is a no-op and the window/timeline populate as normal.
    ipcHandlers.set("get_channel_window", (args) =>
      Promise.resolve(windowResponse(args.channelId)),
    );

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });

    const channel = { id: "live-channel", channelType: "channel" };
    const { root, Harness } = mountChannel(client, channel);
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(Harness),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    assert.equal(
      present(client, channelWindowKey("live-channel")),
      true,
      "mounted fetch must populate the window key",
    );
    assert.deepEqual(
      client.getQueryData(channelMessagesKey("live-channel")),
      [],
      "mounted fetch must populate the (empty) timeline array",
    );

    await act(async () => {
      root.unmount();
    });
    client.clear();
  });
});
