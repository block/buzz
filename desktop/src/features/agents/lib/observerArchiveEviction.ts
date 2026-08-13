/**
 * Pure LRU-with-pins eviction policy for the channel-scoped observer archive.
 *
 * The archive store (`observerRelayStore`) keys paged history by
 * `${normalizedAgentPubkey}:${channelId}`, so a single channel observed for
 * several agents occupies several map entries. Memory pressure, however, scales
 * with the number of distinct *channels* whose scroll-back is retained — the
 * user opens channels, not (agent, channel) pairs. This module therefore evicts
 * at channel granularity: when the count of distinct retained channels exceeds
 * `cap`, the least-recently-accessed unpinned channels are dropped whole (every
 * agent entry for that channel). Eviction is not data loss — the next visit
 * re-hydrates the channel from SQLite through the normal paging path.
 *
 * Pins are inviolable: a pinned channel is never selected for eviction, even
 * when pins alone exceed `cap`. A pin marks a channel whose panel is mounted and
 * whose eviction would blank a live view, so the cap is enforced only against
 * unpinned channels.
 *
 * Pure and React/Tauri-free so the selection logic is unit-testable in
 * isolation, mirroring the `archivePagingState` extraction.
 */

export interface ArchiveEntryMeta {
  /** Composite map key `${normalizedAgentPubkey}:${channelId}`. */
  key: string;
  /** Channel the entry belongs to; the unit of eviction. */
  channelId: string;
  /** Monotonic access counter; higher = more recently read or written. */
  accessSeq: number;
}

/**
 * Select the composite map keys to evict so that no more than `cap` distinct
 * unpinned channels are retained. Returns the keys for every entry belonging to
 * an evicted channel; pinned channels never appear in the result.
 *
 * @param entries        One meta per archive map entry (composite key granularity).
 * @param pinnedChannels Channel ids whose entries must be retained regardless of cap.
 * @param cap            Maximum distinct unpinned channels to keep.
 */
export function selectArchiveEvictionKeys(
  entries: readonly ArchiveEntryMeta[],
  pinnedChannels: ReadonlySet<string>,
  cap: number,
): string[] {
  // Recency of each channel = the most recent access across all its entries,
  // so touching any agent's view of a channel refreshes the whole channel.
  const channelRecency = new Map<string, number>();
  for (const entry of entries) {
    const prev = channelRecency.get(entry.channelId);
    if (prev === undefined || entry.accessSeq > prev) {
      channelRecency.set(entry.channelId, entry.accessSeq);
    }
  }

  // Only unpinned channels are eviction candidates; pins claim retained slots.
  const unpinned: Array<{ channelId: string; recency: number }> = [];
  let pinnedPresent = 0;
  for (const [channelId, recency] of channelRecency) {
    if (pinnedChannels.has(channelId)) {
      pinnedPresent += 1;
    } else {
      unpinned.push({ channelId, recency });
    }
  }

  const unpinnedBudget = Math.max(0, cap - pinnedPresent);
  if (unpinned.length <= unpinnedBudget) {
    return [];
  }

  // Keep the most-recent `unpinnedBudget` channels; evict the rest. Sort by
  // recency descending, breaking ties by channelId so selection is deterministic.
  unpinned.sort(
    (a, b) =>
      b.recency - a.recency ||
      (a.channelId < b.channelId ? -1 : a.channelId > b.channelId ? 1 : 0),
  );
  const evictChannels = new Set(
    unpinned.slice(unpinnedBudget).map((channel) => channel.channelId),
  );

  // Expand evicted channels back to every composite map key they cover.
  const keys: string[] = [];
  for (const entry of entries) {
    if (evictChannels.has(entry.channelId)) {
      keys.push(entry.key);
    }
  }
  return keys;
}
