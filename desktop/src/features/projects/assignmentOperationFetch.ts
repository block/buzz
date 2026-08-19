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
 * The relay clamps every REQ page to this many rows regardless of the
 * requested `limit` (`DEFAULT_MAX_PAGE_LIMIT` in `crates/buzz-db/src/event.rs`).
 * A single second denser than this is unreachable through NIP-01 pagination,
 * so the loop below reports it as an error instead of silently dropping
 * operations.
 */
const RELAY_MAX_PAGE_LIMIT = 1_000;

/** Issue ids per relay query. Each id adds one JSONB containment clause to the
 * relay's SQL, so batches are kept small enough to stay cheap while still
 * collapsing typical projects into a single query. */
const ISSUE_ID_CHUNK_SIZE = 100;

function isAssignmentOperation(event: RelayEvent): boolean {
  return event.tags.some(
    (tag) =>
      tag[0] === "t" &&
      (tag[1] === ISSUE_ASSIGNMENT_LABEL ||
        tag[1] === ISSUE_UNASSIGNMENT_LABEL),
  );
}

/**
 * Loads every assignment/unassignment operation for the given issues,
 * paginating to exhaustion instead of trusting a bounded comment window.
 *
 * Why: assignment state is reduced from kind:1 operations (`t: assignment` /
 * `t: unassignment`), but the general comment fetches are bounded (500 per
 * repo in `hooks.ts`, 2,000 shared in `projectWorkItems.ts`). Once newer
 * comments push an older operation out of that window, its assignee silently
 * vanishes from the issue — and a later self-service operation can reduce
 * against the wrong `prior` head.
 *
 * The filter deliberately carries ONLY constraints the relay pushes into SQL
 * before applying `LIMIT`: kinds, `#e`, `until`, `limit` (see
 * `filter_fully_pushable` in `crates/buzz-relay/src/handlers/req.rs`). Tag
 * filters like `#t`/`#a` are post-filtered in Rust AFTER the SQL `LIMIT`, so
 * including them would make a short page meaningless — the newest N candidate
 * rows could all be post-filtered away while older matches remain, and the
 * loop would declare exhaustion having seen nothing. Instead the query walks
 * the full comment stream of the given issues (`#e` is pushed via JSONB
 * containment) and the assignment labels are filtered locally.
 *
 * Pagination uses an inclusive `until` cursor with id-level dedupe. The relay
 * orders `(created_at DESC, id ASC)`, so a full page whose oldest timestamp
 * equals the cursor means a single second denser than the page: the loop
 * escalates `limit` to the relay's hard page clamp once, and if the second is
 * denser than even that, throws — the caller surfaces a failed assignments
 * section instead of silently losing operations. NIP-01 filters cannot
 * express the relay's composite `(created_at, id)` keyset cursor, so this is
 * the strongest client-only guarantee available.
 */
export async function fetchAssignmentOperationEvents(
  issueIds: string[],
  fetchEvents: (
    filter: FetchEventsInput,
  ) => Promise<RelayEvent[]> = relayClient.fetchEvents.bind(relayClient),
): Promise<RelayEvent[]> {
  if (issueIds.length === 0) return [];
  const chunks: string[][] = [];
  for (let i = 0; i < issueIds.length; i += ISSUE_ID_CHUNK_SIZE) {
    chunks.push(issueIds.slice(i, i + ISSUE_ID_CHUNK_SIZE));
  }
  const pages = await Promise.all(
    chunks.map((chunk) => fetchIssueCommentsExhaustively(chunk, fetchEvents)),
  );
  const seen = new Map<string, RelayEvent>();
  for (const page of pages) {
    for (const event of page) {
      if (isAssignmentOperation(event) && !seen.has(event.id)) {
        seen.set(event.id, event);
      }
    }
  }
  return [...seen.values()];
}

async function fetchIssueCommentsExhaustively(
  issueIds: string[],
  fetchEvents: (filter: FetchEventsInput) => Promise<RelayEvent[]>,
): Promise<RelayEvent[]> {
  const seen = new Map<string, RelayEvent>();
  let limit = ASSIGNMENT_PAGE_LIMIT;
  let until: number | undefined;
  for (;;) {
    const page = await fetchEvents({
      kinds: [KIND_TEXT_NOTE],
      "#e": issueIds,
      limit,
      ...(until === undefined ? {} : { until }),
    });
    for (const event of page) {
      if (!seen.has(event.id)) seen.set(event.id, event);
    }
    // Only SQL-pushed constraints are in the filter, so a short page is a
    // true end-of-results signal.
    if (page.length < limit) break;
    const oldest = Math.min(...page.map((event) => event.created_at));
    if (until === undefined || oldest < until) {
      until = oldest;
      continue;
    }
    // Full page and the inclusive cursor cannot advance: every row shares
    // the cursor second. Widen to the relay's hard clamp so the whole second
    // fits in one page; beyond that, no NIP-01 filter can reach the rest.
    if (limit < RELAY_MAX_PAGE_LIMIT) {
      limit = RELAY_MAX_PAGE_LIMIT;
      continue;
    }
    throw new Error(
      "Could not load assignment history: more than a full relay page of " +
        "issue comments share one timestamp.",
    );
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
