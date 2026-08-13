import type { QueryClient, QueryKey } from "@tanstack/react-query";

/**
 * Per-unit generation counter for the bounded message-window cache. A "unit" is
 * one retained timeline: a channel (its `channel-window` + `channel-messages`
 * keys, keyed by channel id) or a thread subtree (keyed by channel + root). The
 * counter is advanced when a unit is evicted (see `enforceMessageWindowBounds`).
 *
 * The fetch-pin keeps a unit from being evicted *while its query is fetching*,
 * but a deferred background writer — reaction/aux hydration, ancestor loads,
 * the edit/delete mutation success handlers — fires after its own promise
 * settles with no pin and no abort signal. In the window between "the fetch
 * settled, the pin dropped, the resweep evicted the unit" and "the deferred
 * write lands", an `(current = []) => …` updater would silently resurrect the
 * evicted key, or (on an A→B→A revisit) merge stale events onto the re-fetched
 * timeline. This generation fence is that backstop: a writer captures the
 * unit's generation before its async work and commits through
 * `updateRetainedMessageUnit`, which drops the write if the generation moved.
 */
const unitGenerations = new Map<string, number>();

/** Read a unit's current generation; a unit never yet evicted reads 0. */
export function messageUnitGeneration(unitId: string): number {
  return unitGenerations.get(unitId) ?? 0;
}

/**
 * Advance a unit's generation on eviction, invalidating every in-flight
 * deferred write that captured an earlier value.
 */
export function bumpMessageUnitGeneration(unitId: string): void {
  unitGenerations.set(unitId, messageUnitGeneration(unitId) + 1);
}

/** Test-only: clear all counters so cases don't leak generations across each other. */
export function resetMessageUnitGenerations(): void {
  unitGenerations.clear();
}

/**
 * Guarded deferred write into a retained message query. Commits `updater` only
 * when BOTH hold:
 *   - the target key is still present — an evicted key reads back absent, and
 *     React Query resurrects it from an `(current = []) => …` updater, so a bare
 *     merge would rebuild the timeline this cache bound just dropped;
 *   - the unit's generation still equals `generationAtStart` — a mismatch means
 *     the unit was evicted (and possibly re-fetched under a fresh generation)
 *     while the caller was awaiting, so the write carries stale events.
 * Otherwise the write is dropped. A present-but-dataless query (mid-fetch, no
 * data yet) is left untouched; its in-flight fetch produces the authoritative
 * window.
 */
export function updateRetainedMessageUnit<T>(
  queryClient: QueryClient,
  key: QueryKey,
  unitId: string,
  generationAtStart: number,
  updater: (current: T) => T,
): void {
  if (
    queryClient.getQueryState(key) === undefined ||
    messageUnitGeneration(unitId) !== generationAtStart
  ) {
    return;
  }
  queryClient.setQueryData<T>(key, (current) =>
    current === undefined ? current : updater(current),
  );
}
