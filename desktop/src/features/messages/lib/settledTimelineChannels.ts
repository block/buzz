/**
 * Session registry of channels whose timeline has settled at least once.
 *
 * The per-mount loading latch (resolveTimelineLoadingLatch) resets on every
 * channel switch, so a revisit past the messages query's staleTime used to
 * refetch on mount with the "unsettled" skeleton branch active — flashing a
 * skeleton over a fully cached timeline for the whole relay round-trip. A
 * channel that settled once this session has an authoritative window in the
 * query cache (live updates merge into it while away), so revisits render
 * stale-while-revalidate instead.
 *
 * Community-scoped module state: reset via resetCommunityState()
 * (useCommunityInit.ts), like every other community-scoped singleton.
 */

const settledChannelIds = new Set<string>();

export function markTimelineSettledThisSession(channelId: string): void {
  settledChannelIds.add(channelId);
}

export function hasTimelineSettledThisSession(channelId: string): boolean {
  return settledChannelIds.has(channelId);
}

export function resetSettledTimelineChannels(): void {
  settledChannelIds.clear();
}
