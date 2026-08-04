import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

const PROJECT_ENUMERATION_PAGE_SIZE = 500;

type ProjectEventFilter = {
  kinds: number[];
  limit: number;
  since?: number;
  until?: number;
};

type FetchProjectEventPage = (
  filter: ProjectEventFilter,
) => Promise<RelayEvent[]>;

/**
 * Enumerates a NIP-01 websocket filter with the boundary-bucket drain required
 * by NIP-MP. A bare `until` cursor cannot safely advance until every event in
 * the oldest returned second has been retrieved.
 */
export async function enumerateProjectEvents(
  fetchPage: FetchProjectEventPage,
  kinds: number[],
  pageSize: number,
): Promise<RelayEvent[]> {
  if (!Number.isSafeInteger(pageSize) || pageSize <= 0) {
    throw new Error(
      "Project enumeration page size must be a positive integer.",
    );
  }

  const eventsById = new Map<string, RelayEvent>();
  let until: number | undefined;

  for (;;) {
    const page = await fetchPage({
      kinds,
      limit: pageSize,
      ...(until === undefined ? {} : { until }),
    });
    for (const event of page) eventsById.set(event.id, event);
    if (page.length < pageSize) return [...eventsById.values()];

    const oldest = Math.min(...page.map((event) => event.created_at));
    const boundary = await fetchPage({
      kinds,
      limit: pageSize,
      since: oldest,
      until: oldest,
    });
    for (const event of boundary) eventsById.set(event.id, event);
    if (boundary.length >= pageSize) {
      throw new Error(
        "The relay cannot exhaustively enumerate projects because too many events share one timestamp.",
      );
    }
    if (oldest <= 0) return [...eventsById.values()];
    until = oldest - 1;
  }
}

export function fetchProjectEventsExhaustively(
  kinds: number[],
  pageSize = PROJECT_ENUMERATION_PAGE_SIZE,
): Promise<RelayEvent[]> {
  return enumerateProjectEvents(
    (filter) => relayClient.fetchEvents(filter),
    kinds,
    pageSize,
  );
}
