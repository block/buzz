/**
 * Mounted-hook eviction-fence tests for the two DEFERRED background writers
 * that are React hooks (the reaction- and aux-hydration writers are plain
 * async functions, fenced end-to-end in boundMessageWindows.test.mjs):
 *
 *   - useLoadMissingAncestors — its `getEventById` fetch settles after the
 *     resweep evicts the channel; the deferred `mergeMessages` write must be
 *     dropped by the generation fence (`updateRetainedMessageUnit`).
 *   - useDeleteMessageMutation — its `delete_message` mutation resolves after
 *     the resweep; the deferred `onSuccess` filter must be dropped likewise.
 *
 * Both mount the REAL production hook (its useEffect / mutation wiring, its
 * generation capture, and its guarded commit) so the test fails if the
 * generation capture or the `updateRetainedMessageUnit` commit is removed.
 * Ordering is Paul's: fetch in flight → resweep evicts the channel → fetch
 * settles → deferred write fires → BOTH keys must stay absent (no
 * `(current) => …` resurrection of the timeline the cache just bound away).
 *
 * DOM shim: react-dom/client needs a minimal DOM; installDOMShim (shared
 * observed-unread harness) provides it, matching the other mounted-hook suites.
 * Tauri IPC is intercepted at globalThis.__TAURI_INTERNALS__.invoke by command
 * name (get_event, delete_message), the pattern from
 * useLoadArchivedObserverEvents.test.mjs.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { installDOMShim } from "@/features/channels/observedUnreadTestHarness.mjs";

installDOMShim();

// ── Tauri IPC interceptor (installed before any module importing tauri.ts) ────

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

function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
function clearIpcHandlers() {
  ipcHandlers.clear();
}

// ── Production imports (after shim, after IPC stub) ───────────────────────────

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
import { useLoadMissingAncestors } from "./useLoadMissingAncestors.ts";
import { useDeleteMessageMutation } from "./hooks.ts";

const CHAN_CAP = MAX_RETAINED_MESSAGE_CHANNELS;
const EVICTED = "evicted-channel";

/** A relay event with the channel `h` tag; `reply`/`root` tags optional. */
function event(id, createdAt, extraTags = []) {
  return {
    id: id.padEnd(64, "0"),
    pubkey: "a".repeat(64),
    created_at: createdAt,
    kind: 9,
    tags: [["h", EVICTED], ...extraTags],
    content: id,
    sig: "b".repeat(128),
  };
}

/** Seed the evicted channel's keys plus CHAN_CAP fresher channels above it. */
function seedOverCap(client, evictedMessages) {
  for (let i = 0; i < CHAN_CAP; i += 1) {
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
  client.setQueryData(channelWindowKey(EVICTED), emptyChannelWindowStore(), {
    updatedAt: 0,
  });
  client.setQueryData(channelMessagesKey(EVICTED), evictedMessages, {
    updatedAt: 0,
  });
}

function hasEvictedChannel(client) {
  return (
    client.getQueryData(channelWindowKey(EVICTED)) !== undefined ||
    client.getQueryData(channelMessagesKey(EVICTED)) !== undefined
  );
}

async function settle() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 5));
  });
}

describe("deferred hook writers — eviction fence", () => {
  beforeEach(() => {
    resetMessageUnitGenerations();
    clearIpcHandlers();
  });

  it("test_deferred_ancestor_load_does_not_resurrect_evicted_channel", async () => {
    const ancestorId = "c".repeat(64);
    // A resolved reply whose `reply` tag points at a missing ancestor — the
    // hook fetches that ancestor via getEventById.
    const reply = event("reply", 10, [["e", ancestorId, "", "reply"]]);

    let releaseFetch;
    setIpcHandler(
      "get_event",
      () =>
        new Promise((resolve) => {
          // getEventById JSON.parses the returned string. The ancestor carries
          // the evicted channel's `h` tag so it passes the hook's channel-match
          // guard — proving it is the GENERATION fence, not a channel mismatch,
          // that drops the write.
          releaseFetch = () => resolve(JSON.stringify(event("ancestor", 5)));
        }),
    );

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });
    seedOverCap(client, [reply]);

    const channel = { id: EVICTED, channelType: "channel" };
    function Harness() {
      useLoadMissingAncestors(channel, [reply]);
      return null;
    }
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(Harness),
        ),
      );
    });
    await settle();

    // Resweep while the ancestor fetch is still pending: the over-cap channel
    // is evicted (generation bumped, both keys removed).
    enforceMessageWindowBounds(client);
    assert.equal(
      hasEvictedChannel(client),
      false,
      "channel evicted mid-ancestor-fetch",
    );

    // The in-flight fetch settles and the deferred ancestor merge attempts its
    // write against the stale captured generation.
    await act(async () => {
      releaseFetch();
      await new Promise((r) => setTimeout(r, 5));
    });

    assert.equal(
      client.getQueryData(channelMessagesKey(EVICTED)),
      undefined,
      "ancestor merge must not resurrect the messages key",
    );
    assert.equal(
      client.getQueryData(channelWindowKey(EVICTED)),
      undefined,
      "ancestor merge must not resurrect the window key",
    );

    await act(async () => {
      root.unmount();
    });
  });

  it("test_deferred_delete_success_does_not_resurrect_evicted_channel", async () => {
    const targetId = "d".repeat(64);

    let releaseDelete;
    setIpcHandler(
      "delete_message",
      () =>
        new Promise((resolve) => {
          releaseDelete = () => resolve(null);
        }),
    );

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });
    seedOverCap(client, [event("target", 10)]);

    const channel = { id: EVICTED, channelType: "channel" };
    const mutationRef = { current: null };
    function Harness() {
      mutationRef.current = useDeleteMessageMutation(channel);
      return null;
    }
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(Harness),
        ),
      );
    });

    // Fire the delete: onMutate captures the (unevicted) generation, then the
    // mutationFn awaits the deferred delete_message IPC.
    let mutatePromise;
    await act(async () => {
      mutatePromise = mutationRef.current.mutateAsync({ eventId: targetId });
      await new Promise((r) => setTimeout(r, 5));
    });

    // Resweep while the delete is in flight: the channel is evicted.
    enforceMessageWindowBounds(client);
    assert.equal(
      hasEvictedChannel(client),
      false,
      "channel evicted mid-delete",
    );

    // The delete resolves; onSuccess's filter must be fenced.
    await act(async () => {
      releaseDelete();
      await mutatePromise;
    });

    assert.equal(
      client.getQueryData(channelMessagesKey(EVICTED)),
      undefined,
      "delete onSuccess must not resurrect the messages key",
    );
    assert.equal(
      client.getQueryData(channelWindowKey(EVICTED)),
      undefined,
      "delete onSuccess must not resurrect the window key",
    );

    await act(async () => {
      root.unmount();
    });
  });
});
