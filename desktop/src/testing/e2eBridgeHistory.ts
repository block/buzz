import type { RelayEvent } from "@/shared/api/types";
import { matchFilter, type Filter } from "nostr-tools/filter";

/** Stored-event replay for mock REQs: per-filter limits, union, then one EOSE. */
export function selectMockHistory(
  channels: ReadonlyMap<string, RelayEvent[]>,
  filters: Filter[],
): RelayEvent[] {
  const selected = new Map<string, RelayEvent>();
  for (const filter of filters) {
    // The relay scopes reactions/deletions through their stored channel, even
    // when their wire tags only name an event. Keep emitted tags unchanged.
    const { "#h": channelIds, ...eventFilter } = filter;
    const candidates = [...channels]
      .filter(([channelId]) => !channelIds || channelIds.includes(channelId))
      .flatMap(([, events]) => events);
    const ordered = [
      ...new Map(candidates.map((event) => [event.id, event])).values(),
    ].sort((a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id));
    for (const event of ordered
      .filter(
        (event) =>
          matchFilter(
            event.tags.some(([tag]) => tag === "h") ? filter : eventFilter,
            event,
          ) &&
          (filter.since === undefined || event.created_at >= filter.since) &&
          (filter.until === undefined || event.created_at <= filter.until),
      )
      .slice(0, filter.limit ?? 50)) {
      selected.set(event.id, event);
    }
  }
  return [...selected.values()].sort(
    (a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id),
  );
}
