import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_TEXT_NOTE } from "@/shared/constants/kinds";
import {
  ISSUE_ASSIGNMENT_LABEL,
  ISSUE_UNASSIGNMENT_LABEL,
} from "./projectIssues.mjs";

type FetchEventsInput = Parameters<(typeof relayClient)["fetchEvents"]>[0];

const ASSIGNMENT_PAGE_LIMIT = 500;

/**
 * Loads every assignment/unassignment operation for the given repositories,
 * paginating to exhaustion instead of trusting a bounded comment window.
 *
 * Why: assignment state is reduced from kind:1 operations (`t: assignment` /
 * `t: unassignment`), but the general comment fetches are bounded (500 per
 * repo in `hooks.ts`, 2,000 shared in `projectWorkItems.ts`). Once newer
 * comments push an older operation out of that window, its assignee silently
 * vanishes from the issue — and a later self-service operation can reduce
 * against the wrong `prior` head. Operations are rare relative to comments,
 * so a label-filtered exhaustive query stays small.
 *
 * Pagination uses an inclusive `until` cursor with id-level dedupe and stops
 * when a page yields nothing unseen. A bare `until` cursor cannot escape a
 * single second denser than one full page (see `ChannelPageCursor` in
 * shared/api/types.ts), but that would require >500 assignment operations
 * signed within one second — not producible by any supported writer.
 */
export async function fetchAssignmentOperationEvents(
  repoAddresses: string[],
  fetchEvents: (
    filter: FetchEventsInput,
  ) => Promise<RelayEvent[]> = relayClient.fetchEvents.bind(relayClient),
): Promise<RelayEvent[]> {
  if (repoAddresses.length === 0) return [];
  const seen = new Map<string, RelayEvent>();
  let until: number | undefined;
  for (;;) {
    const page = await fetchEvents({
      kinds: [KIND_TEXT_NOTE],
      "#a": repoAddresses,
      "#t": [ISSUE_ASSIGNMENT_LABEL, ISSUE_UNASSIGNMENT_LABEL],
      limit: ASSIGNMENT_PAGE_LIMIT,
      ...(until === undefined ? {} : { until }),
    });
    let unseen = 0;
    for (const event of page) {
      if (!seen.has(event.id)) {
        seen.set(event.id, event);
        unseen += 1;
      }
    }
    if (unseen === 0 || page.length < ASSIGNMENT_PAGE_LIMIT) break;
    until = Math.min(...page.map((event) => event.created_at));
  }
  return [...seen.values()];
}

/** Merge two event lists, dropping duplicates by event id. */
export function mergeEventsById(
  base: RelayEvent[],
  extra: RelayEvent[],
): RelayEvent[] {
  const ids = new Set(base.map((event) => event.id));
  return [...base, ...extra.filter((event) => !ids.has(event.id))];
}
