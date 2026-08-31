import type { RelayEvent } from "@/shared/api/types";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";

/** HTTP /query keyset extension; do not send this through a NIP-01 socket. */
export type HostHistoryFilter = RelaySubscriptionFilter & {
  before_id?: string;
};

/** Exhaust exact, primary-backed host pages before making a publication decision. */
export async function fetchHostHistory(
  fetchPage: (filter: HostHistoryFilter) => Promise<RelayEvent[]>,
  filter: HostHistoryFilter,
  check: () => void,
): Promise<RelayEvent[]> {
  const events: RelayEvent[] = [];
  const seen = new Set<string>();
  let cursor: RelayEvent | undefined;
  for (;;) {
    check();
    const page = await fetchPage({
      ...filter,
      ...(cursor ? { until: cursor.created_at, before_id: cursor.id } : {}),
    });
    check();
    if (page.length > filter.limit || filter.limit <= 0)
      throw new Error("Invalid host history page");
    // HTTP result order is not part of the client contract. Sort using the
    // database's DESC timestamp / ASC event-ID keyset order.
    page.sort(
      (a, b) =>
        b.created_at - a.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
    );
    for (const event of page) {
      if (
        !/^[0-9a-f]{64}$/.test(event.id) ||
        !Number.isSafeInteger(event.created_at) ||
        event.created_at < 0 ||
        seen.has(event.id) ||
        (cursor &&
          (event.created_at > cursor.created_at ||
            (event.created_at === cursor.created_at && event.id <= cursor.id)))
      )
        throw new Error("Host history cursor did not advance");
      seen.add(event.id);
      events.push(event);
    }
    if (page.length < filter.limit) return events;
    cursor = page[page.length - 1];
  }
}
