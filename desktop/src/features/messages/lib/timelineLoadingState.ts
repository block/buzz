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
): boolean {
  if (status.isPending) {
    return true;
  }
  // A fetch is in flight; keep loading while what we'd show is a placeholder or
  // still empty. Once real rows are present we are loaded, even mid-refetch.
  return (
    status.isFetching &&
    (status.isPlaceholderData || (status.dataLength ?? 0) === 0)
  );
}
