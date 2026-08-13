import type { Query, QueryClient } from "@tanstack/react-query";

import type { ChannelWindowStore } from "./channelWindowStore";
import { bumpMessageUnitGeneration } from "./messageUnitGuard";
import { clearChannelRenderScopedReactionHydration } from "./renderScopedReactions";
import type { RelayEvent } from "@/shared/api/types";
import {
  selectMessageWindowEvictionUnits,
  type MessageWindowUnit,
} from "./messageWindowEviction";

/**
 * Maximum number of distinct unpinned channel timelines and thread subtrees
 * retained in the query cache. Channels and threads are bounded independently
 * so a burst of open threads never evicts channel scrollback and vice versa.
 * A revisit re-fetches from the relay (cache miss, not data loss).
 */
export const MAX_RETAINED_MESSAGE_CHANNELS = 12;
export const MAX_RETAINED_MESSAGE_THREADS = 24;

const CHANNEL_WINDOW = "channel-window";
const CHANNEL_MESSAGES = "channel-messages";
const THREAD_REPLIES = "thread-replies";

/** A cached message query keyed by its parsed unit id, tagged by kind. */
type ClassifiedQuery = {
  query: Query;
  unitId: string;
  kind: "channel" | "thread";
};

/**
 * Classify a cache query as a channel-timeline key, a thread key, or neither.
 * Channel units fold both `channel-window` and `channel-messages` under the
 * channel id so they evict together; thread units key on channel + root.
 */
function classifyQuery(query: Query): ClassifiedQuery | null {
  const key = query.queryKey;
  const head = key[0];
  if (
    (head === CHANNEL_WINDOW || head === CHANNEL_MESSAGES) &&
    typeof key[1] === "string"
  ) {
    return { query, unitId: key[1], kind: "channel" };
  }
  if (
    head === THREAD_REPLIES &&
    typeof key[1] === "string" &&
    typeof key[2] === "string"
  ) {
    return { query, unitId: `${key[1]}\u0000${key[2]}`, kind: "thread" };
  }
  return null;
}

/** Whether a query's cached data holds an optimistic pending send. */
function holdsPendingSend(query: Query): boolean {
  const data = query.state.data;
  if (Array.isArray(data)) {
    return (data as RelayEvent[]).some((event) => event.pending);
  }
  // A channel-window store keeps optimistic sends in its live overlay — the
  // seam that catches a send-from-thread landing on a non-visible channel.
  const overlay = (data as ChannelWindowStore | undefined)?.liveOverlay;
  return Array.isArray(overlay) && overlay.some((event) => event.pending);
}

/**
 * Fold classified queries into eviction units. A unit is pinned when ANY of
 * its queries has a mounted observer (the view is on screen — the "none"
 * placeholder channel is inactive and so never pins), is mid-fetch (its
 * authoritative window is still loading), or holds a pending send. Recency is
 * the freshest `dataUpdatedAt` across the unit's queries.
 */
function foldUnits(queries: ClassifiedQuery[]): Map<string, MessageWindowUnit> {
  const units = new Map<string, MessageWindowUnit>();
  for (const { query, unitId } of queries) {
    const pinned =
      query.isActive() ||
      query.state.fetchStatus === "fetching" ||
      holdsPendingSend(query);
    const recency = query.state.dataUpdatedAt;
    const existing = units.get(unitId);
    if (existing) {
      existing.pinned ||= pinned;
      existing.recency = Math.max(existing.recency, recency);
    } else {
      units.set(unitId, { unitId, pinned, recency });
    }
  }
  return units;
}

/**
 * Enforce the retained-timeline bound on the query cache. Collects every
 * channel and thread query, folds them into pinned/recency units, selects the
 * least-recently-updated unpinned units beyond the caps, and removes their
 * queries. Channels and threads are bounded independently.
 *
 * Removal (not invalidation) is deliberate: a removed key reads back as absent,
 * so a revisit re-fetches fresh via the unconditional post-subscribe refetch.
 * Deferred background writers (reaction/aux hydration, ancestor loads, mutation
 * success handlers) fire after their own promise settles with no pin, so each
 * commits through `updateRetainedMessageUnit`; bumping the evicted unit's
 * generation here fences any such write that is already in flight, and clearing
 * a channel's reaction-hydration claims lets its revisit re-hydrate from
 * scratch instead of treating every id as already hydrated.
 */
export function enforceMessageWindowBounds(queryClient: QueryClient): void {
  const channels: ClassifiedQuery[] = [];
  const threads: ClassifiedQuery[] = [];
  for (const query of queryClient.getQueryCache().getAll()) {
    const classified = classifyQuery(query);
    if (!classified) continue;
    (classified.kind === "channel" ? channels : threads).push(classified);
  }

  const evictChannels = new Set(
    selectMessageWindowEvictionUnits(
      [...foldUnits(channels).values()],
      MAX_RETAINED_MESSAGE_CHANNELS,
    ),
  );
  const evictThreads = new Set(
    selectMessageWindowEvictionUnits(
      [...foldUnits(threads).values()],
      MAX_RETAINED_MESSAGE_THREADS,
    ),
  );
  if (evictChannels.size === 0 && evictThreads.size === 0) return;

  for (const unitId of evictChannels) {
    bumpMessageUnitGeneration(unitId);
    clearChannelRenderScopedReactionHydration(unitId);
  }
  for (const unitId of evictThreads) {
    bumpMessageUnitGeneration(unitId);
  }

  for (const { query, unitId, kind } of [...channels, ...threads]) {
    const evict = kind === "channel" ? evictChannels : evictThreads;
    if (evict.has(unitId)) {
      queryClient.getQueryCache().remove(query);
    }
  }
}

// Per-client coalescing flag for `scheduleMessageWindowSweep`. A WeakSet keyed
// by the client needs no teardown — a client that is garbage-collected drops
// its flag with it, and a pending microtask always runs.
const sweepScheduled = new WeakSet<QueryClient>();

/**
 * Coalesce a bounds sweep to one run per microtask per client. Both the cache
 * subscription (a key was added, an observer unmounted, or a fetch settled) and
 * the send mutation (its optimistic pending-send pin drained) funnel through
 * here, so every distinct "a pin may have been released" signal shares one
 * generic resweep — a fetch pin and a mutation pin are not fixed separately.
 * A channel open adds two keys at once and both land in the same microtask, so
 * the sweep still runs once.
 */
export function scheduleMessageWindowSweep(queryClient: QueryClient): void {
  if (sweepScheduled.has(queryClient)) return;
  sweepScheduled.add(queryClient);
  queueMicrotask(() => {
    sweepScheduled.delete(queryClient);
    enforceMessageWindowBounds(queryClient);
  });
}
