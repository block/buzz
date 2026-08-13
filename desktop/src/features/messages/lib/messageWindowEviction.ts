/**
 * Pure LRU-with-pins eviction policy for the message query-cache windows.
 *
 * The channel timeline lives in the React Query cache as two keys per channel
 * (`channel-window` holding the authoritative window store and
 * `channel-messages` holding the flattened render array) plus one
 * `thread-replies` key per open thread root. Every visited channel or thread
 * leaves its keys in the cache for `gcTime` (one hour) after the view unmounts,
 * so a session that visits many channels accumulates every timeline's window
 * store and flattened array at once — a renderer memory leak behind the v0.5.9
 * lag. React Query's `gcTime` bounds *how long* an inactive query lingers, not
 * *how many* linger concurrently; this policy adds the missing concurrency
 * bound.
 *
 * Eviction is at unit granularity — a channel unit owns both of its keys, a
 * thread unit owns its single key — because the two channel keys are one
 * timeline: dropping the window store while keeping the flattened array leaves
 * a render array that live merges keep re-growing against no backing store.
 *
 * Pins are inviolable: a unit is never evicted while any of its queries has a
 * mounted observer (the view is on screen) or holds an optimistic pending send
 * (dropping it would lose an in-flight message the relay has not yet echoed).
 * The cap is enforced only against unpinned units, even when pins alone exceed
 * it.
 *
 * Pure and React/React-Query-free so the selection logic is unit-testable in
 * isolation, mirroring the Target 1 `observerArchiveEviction` extraction.
 */

export interface MessageWindowUnit {
  /** Stable identifier for the eviction unit (channel id, or channel+root). */
  unitId: string;
  /** Most recent `dataUpdatedAt` across the unit's queries; higher = fresher. */
  recency: number;
  /** True when the unit must be retained: mounted observer or pending send. */
  pinned: boolean;
}

/**
 * Select the unit ids to evict so that no more than `cap` distinct unpinned
 * units are retained. Pinned units never appear in the result.
 *
 * @param units One entry per eviction unit (channel or thread).
 * @param cap   Maximum distinct unpinned units to keep.
 */
export function selectMessageWindowEvictionUnits(
  units: readonly MessageWindowUnit[],
  cap: number,
): string[] {
  const unpinned = units.filter((unit) => !unit.pinned);
  const pinnedCount = units.length - unpinned.length;
  const unpinnedBudget = Math.max(0, cap - pinnedCount);
  if (unpinned.length <= unpinnedBudget) {
    return [];
  }

  // Keep the most-recent `unpinnedBudget` units; evict the rest. Sort by
  // recency descending, breaking ties by unitId so selection is deterministic.
  const evictable = [...unpinned].sort(
    (a, b) =>
      b.recency - a.recency ||
      (a.unitId < b.unitId ? -1 : a.unitId > b.unitId ? 1 : 0),
  );
  return evictable.slice(unpinnedBudget).map((unit) => unit.unitId);
}
