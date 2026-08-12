/**
 * Pure decision for "is the channel timeline still doing its initial load."
 *
 * Extracted so the windows below are covered by the lib `*.test.mjs` suite.
 * The trap: `data !== undefined` looks like "loaded" but the per-channel query
 * cache is seeded early — by a stale `placeholderData` on revisit, and by the
 * live subscription's `setQueryData` — before the authoritative history fetch
 * settles. Treating that as loaded flashes the channel intro/empty state over a
 * list that is about to stream in.
 */
export type TimelineQueryStatus = {
  isPending: boolean;
  isFetching: boolean;
  isPlaceholderData: boolean;
  dataLength: number | null;
};

export function selectTimelineLoadingState(
  status: TimelineQueryStatus,
  hasSettled = true,
  hasAuthoritativeCache = false,
): boolean {
  if (status.isPending) {
    return true;
  }
  if (!hasSettled) {
    // A populated authoritative window proves these rows came from a previous
    // settled load, not from the live subscription's partial pre-settle seed.
    // Paint them while a stale query revalidates in the background.
    if (hasAuthoritativeCache && (status.dataLength ?? 0) > 0) {
      return false;
    }
    // Placeholder rows are also a previously-settled timeline (for callers
    // that explicitly configure placeholderData).
    if (status.isPlaceholderData && (status.dataLength ?? 0) > 0) {
      return false;
    }
    // Otherwise hold the skeleton for the whole cold load: the live
    // subscription can seed a few rows before the history fetch settles, and
    // painting those as if loaded flashes a near-empty timeline.
    return status.isFetching;
  }
  return (
    status.isFetching &&
    (status.isPlaceholderData || (status.dataLength ?? 0) === 0)
  );
}

/**
 * Monotonic loading latch keyed by channel. Once a channel has settled (loaded),
 * `loadingNow` blipping true again (a background refetch) must not re-show the
 * skeleton — that re-flip is the visible skeleton bounce on entry. A different
 * channel id resets the latch so the new channel loads fresh.
 */
export function resolveTimelineLoadingLatch(
  settledChannelId: string | null,
  activeChannelId: string | null,
  loadingNow: boolean,
): { settledChannelId: string | null; isLoading: boolean } {
  if (activeChannelId === null) {
    return { settledChannelId, isLoading: loadingNow };
  }
  if (settledChannelId === activeChannelId) {
    // Already settled for this channel — stay loaded through refetch blips.
    return { settledChannelId, isLoading: false };
  }
  if (!loadingNow) {
    // First settle for this channel; latch it.
    return { settledChannelId: activeChannelId, isLoading: false };
  }
  return { settledChannelId, isLoading: true };
}
