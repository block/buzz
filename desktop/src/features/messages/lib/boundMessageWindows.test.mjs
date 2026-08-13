/**
 * Store-level tests for enforceMessageWindowBounds against a REAL QueryClient.
 *
 * These exercise the collect → fold → select → remove path at the cache
 * boundary — the layer above the pure policy unit — asserting:
 *   - channels and threads are bounded independently at their caps;
 *   - eviction is least-recently-updated first;
 *   - evicting a channel removes BOTH its window and messages keys together;
 *   - an active channel (mounted observer) is never evicted, even when stale;
 *   - a pending send pins its channel — including the cross-key case where the
 *     optimistic event lands on a non-visible channel-window `liveOverlay`;
 *   - an evicted channel reads back absent → the freshness seam re-fetches;
 *   - the REAL deferred background writers (reaction hydration, aux backfill)
 *     cannot resurrect an evicted channel when their fetch settles after the
 *     resweep drops the unit — the generation fence, exercised end-to-end
 *     against the production writers and a real eviction, not a synthetic
 *     stand-in for the guarded updater.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it, mock } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";

import {
  channelMessagesKey,
  channelWindowKey,
  threadRepliesKey,
} from "./messageQueryKeys.ts";
import {
  emptyChannelWindowStore,
  mergeLiveChannelWindowEvent,
} from "./channelWindowStore.ts";
import {
  enforceMessageWindowBounds,
  MAX_RETAINED_MESSAGE_CHANNELS,
  MAX_RETAINED_MESSAGE_THREADS,
} from "./boundMessageWindows.ts";
import {
  hydrateRenderScopedReactions,
  resetRenderScopedReactionHydration,
} from "./renderScopedReactions.ts";
import { backfillAuxForMessages } from "./auxBackfill.ts";
import { resetMessageUnitGenerations } from "./messageUnitGuard.ts";
import { relayClient } from "@/shared/api/relayClient.ts";

const CHAN_CAP = MAX_RETAINED_MESSAGE_CHANNELS;
const THREAD_CAP = MAX_RETAINED_MESSAGE_THREADS;

let client;

/** A relay event; `pending` marks an un-acked optimistic send. */
function event(id, createdAt, extra = {}) {
  return {
    id: id.padEnd(64, "0"),
    pubkey: "a".repeat(64),
    created_at: createdAt,
    kind: 9,
    tags: [["h", "channel"]],
    content: id,
    sig: "b".repeat(128),
    ...extra,
  };
}

/**
 * Seed a channel's window + messages keys at a given recency. `updatedAt`
 * drives `dataUpdatedAt` so the LRU order is deterministic in tests.
 */
function seedChannel(channelId, updatedAt, { pendingSend = false } = {}) {
  let store = emptyChannelWindowStore();
  if (pendingSend) {
    store = mergeLiveChannelWindowEvent(
      store,
      event(`pending-${channelId}`, updatedAt, { pending: true }),
    );
  }
  client.setQueryData(channelWindowKey(channelId), store, { updatedAt });
  client.setQueryData(channelMessagesKey(channelId), [], { updatedAt });
}

function seedThread(channelId, rootId, updatedAt) {
  client.setQueryData(threadRepliesKey(channelId, rootId), [], { updatedAt });
}

/** Mount an observer so the query reports `isActive()` — a pinned view. */
function mountChannel(channelId) {
  const windowObserver = new QueryObserver(client, {
    queryKey: channelWindowKey(channelId),
    enabled: true,
  });
  const messagesObserver = new QueryObserver(client, {
    queryKey: channelMessagesKey(channelId),
    enabled: true,
  });
  const unsubWindow = windowObserver.subscribe(() => {});
  const unsubMessages = messagesObserver.subscribe(() => {});
  return () => {
    unsubWindow();
    unsubMessages();
  };
}

function hasChannel(channelId) {
  return (
    client.getQueryData(channelWindowKey(channelId)) !== undefined ||
    client.getQueryData(channelMessagesKey(channelId)) !== undefined
  );
}

describe("enforceMessageWindowBounds", () => {
  beforeEach(() => {
    client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity } },
    });
  });

  it("test_channels_up_to_cap_are_all_retained", () => {
    for (let i = 0; i < CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    enforceMessageWindowBounds(client);
    for (let i = 0; i < CHAN_CAP; i += 1) {
      assert.ok(hasChannel(`chan-${i}`), `chan-${i} must be retained at cap`);
    }
  });

  it("test_exceeding_cap_evicts_least_recently_updated_channel", () => {
    for (let i = 0; i <= CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    enforceMessageWindowBounds(client);
    assert.equal(hasChannel("chan-0"), false, "oldest channel evicted");
    for (let i = 1; i <= CHAN_CAP; i += 1) {
      assert.ok(hasChannel(`chan-${i}`), `chan-${i} must survive`);
    }
  });

  it("test_evicting_a_channel_removes_both_window_and_messages_keys", () => {
    for (let i = 0; i <= CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    enforceMessageWindowBounds(client);
    assert.equal(
      client.getQueryData(channelWindowKey("chan-0")),
      undefined,
      "window key gone",
    );
    assert.equal(
      client.getQueryData(channelMessagesKey("chan-0")),
      undefined,
      "messages key gone",
    );
  });

  it("test_active_channel_is_never_evicted_even_when_least_recent", () => {
    // chan-0 is the oldest (recency 0) but mounted → pinned. One extra channel
    // over cap forces an eviction; the next-oldest unpinned (chan-1) goes.
    for (let i = 0; i <= CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    const unmount = mountChannel("chan-0");
    enforceMessageWindowBounds(client);
    assert.ok(hasChannel("chan-0"), "active channel pinned");
    assert.equal(hasChannel("chan-1"), false, "next-oldest unpinned evicted");
    unmount();
  });

  it("test_pending_send_pins_its_channel_via_window_overlay", () => {
    // chan-0 is oldest and inactive but holds an optimistic send in its window
    // overlay (the cross-key send-from-thread seam) → pinned. chan-1 evicted.
    for (let i = 0; i <= CHAN_CAP; i += 1) {
      seedChannel(`chan-${i}`, i, { pendingSend: i === 0 });
    }
    enforceMessageWindowBounds(client);
    assert.ok(hasChannel("chan-0"), "channel with pending send pinned");
    assert.equal(hasChannel("chan-1"), false, "next-oldest unpinned evicted");
  });

  it("test_threads_are_bounded_independently_from_channels", () => {
    // Many channels AND many threads over both caps: each bounded to its own
    // cap, neither starving the other.
    for (let i = 0; i <= CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    for (let i = 0; i <= THREAD_CAP; i += 1) seedThread("c", `root-${i}`, i);
    enforceMessageWindowBounds(client);
    // Oldest of each evicted; newest survive.
    assert.equal(hasChannel("chan-0"), false);
    assert.equal(
      client.getQueryData(threadRepliesKey("c", "root-0")),
      undefined,
      "oldest thread evicted",
    );
    assert.notEqual(
      client.getQueryData(threadRepliesKey("c", `root-${THREAD_CAP}`)),
      undefined,
      "newest thread survives",
    );
  });

  it("test_evicted_channel_reads_absent_so_revisit_refetches", () => {
    for (let i = 0; i <= CHAN_CAP; i += 1) seedChannel(`chan-${i}`, i);
    enforceMessageWindowBounds(client);
    // An evicted channel's keys read back absent, so its messages query has no
    // cached state to short-circuit against and re-fetches fresh on revisit
    // (the post-subscribe refresh runs unconditionally). Removal, not stale
    // retention, is what makes the revisit a cache miss rather than data loss.
    assert.equal(
      client.getQueryState(channelMessagesKey("chan-0")),
      undefined,
      "evicted channel messages key must read absent on revisit",
    );
    assert.equal(
      client.getQueryState(channelWindowKey("chan-0")),
      undefined,
      "evicted channel window key must read absent on revisit",
    );
  });

  it("test_deferred_reaction_hydration_does_not_resurrect_evicted_channel", async () => {
    // Fill to cap, then open one more channel over cap whose reaction fetch is
    // in flight when the resweep runs. Forces Paul's ordering: the hydrate
    // promise settles → the resweep has already evicted the channel → the
    // deferred merge fires → the generation fence must drop it, leaving BOTH
    // keys absent (no `(current = []) => …` resurrection).
    resetMessageUnitGenerations();
    resetRenderScopedReactionHydration();
    const evicted = "reaction-evicted";
    const messageId = "d".repeat(64);
    for (let i = 0; i < CHAN_CAP; i += 1) seedChannel(`keep-${i}`, i + 100);
    // Seed the target as the least-recently-updated so it is the one evicted.
    seedChannel(evicted, 0);
    client.setQueryData(channelMessagesKey(evicted), [event(messageId, 1)], {
      updatedAt: 0,
    });

    let releaseFetch;
    const hydrate = hydrateRenderScopedReactions({
      channelId: evicted,
      messageIds: [messageId],
      queryClient: client,
      deps: {
        fetchReactionEventsForMessages: () =>
          new Promise((resolve) => {
            releaseFetch = () =>
              resolve([
                event("e".repeat(64), 7, {
                  tags: [
                    ["h", "channel"],
                    ["e", messageId],
                  ],
                }),
              ]);
          }),
      },
    });

    // Resweep while the fetch is still pending: the over-cap channel is evicted.
    enforceMessageWindowBounds(client);
    assert.equal(hasChannel(evicted), false, "channel evicted mid-fetch");

    // Now the in-flight fetch settles and the deferred merge attempts its write.
    releaseFetch();
    await hydrate;

    assert.equal(
      client.getQueryData(channelMessagesKey(evicted)),
      undefined,
      "reaction merge must not resurrect the messages key",
    );
    assert.equal(
      client.getQueryData(channelWindowKey(evicted)),
      undefined,
      "reaction merge must not resurrect the window key",
    );
  });

  it("test_deferred_aux_backfill_does_not_resurrect_evicted_channel", async () => {
    // Same ordering for the structural-aux writer: its aux fetch is in flight
    // when the resweep evicts its channel, so its deferred merge must be fenced.
    resetMessageUnitGenerations();
    const evicted = "aux-evicted";
    const messageId = "a".repeat(64);
    for (let i = 0; i < CHAN_CAP; i += 1) seedChannel(`keep-${i}`, i + 100);
    seedChannel(evicted, 0);
    client.setQueryData(channelMessagesKey(evicted), [event(messageId, 1)], {
      updatedAt: 0,
    });

    let releaseAux;
    const auxByRef = mock.method(
      relayClient,
      "fetchAuxEventsByReference",
      () =>
        new Promise((resolve) => {
          releaseAux = () =>
            resolve([
              event("f".repeat(64), 40003, {
                tags: [
                  ["h", "channel"],
                  ["e", messageId],
                ],
              }),
            ]);
        }),
    );
    const auxDeletions = mock.method(
      relayClient,
      "fetchAuxDeletionEventsForAuxEvents",
      async () => [],
    );

    const backfill = backfillAuxForMessages(client, evicted, [
      event(messageId, 9),
    ]);

    enforceMessageWindowBounds(client);
    assert.equal(hasChannel(evicted), false, "channel evicted mid-backfill");

    releaseAux();
    await backfill;

    assert.equal(
      client.getQueryData(channelMessagesKey(evicted)),
      undefined,
      "aux merge must not resurrect the messages key",
    );
    assert.equal(
      client.getQueryData(channelWindowKey(evicted)),
      undefined,
      "aux merge must not resurrect the window key",
    );
    auxByRef.mock.restore();
    auxDeletions.mock.restore();
  });
});
