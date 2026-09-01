/**
 * Mock-mode `search_messages` predicate, mirroring the relay's filter contract.
 *
 * `since`/`until` are NIP-01 bounds and both inclusive. The `before:` operator's
 * exclusivity is encoded upstream by subtracting one second before this helper
 * receives the bound.
 */
type SearchHit = {
  channel_id: string | null;
  pubkey: string;
  created_at: number;
  content: string;
  channel_name: string | null;
};

export function mockSearchHitMatches(
  hit: SearchHit,
  filters: {
    /** Lowercased FTS query; empty matches everything. */
    query: string;
    channelId?: string;
    authorSet: Set<string> | null;
    since?: number;
    until?: number;
  },
): boolean {
  if (filters.channelId && hit.channel_id !== filters.channelId) {
    return false;
  }
  if (filters.authorSet && !filters.authorSet.has(hit.pubkey.toLowerCase())) {
    return false;
  }
  if (filters.since != null && hit.created_at < filters.since) {
    return false;
  }
  if (filters.until != null && hit.created_at > filters.until) {
    return false;
  }
  if (!filters.query) {
    return true;
  }
  return (
    hit.content.toLowerCase().includes(filters.query) ||
    (hit.channel_name?.toLowerCase().includes(filters.query) ?? false)
  );
}
