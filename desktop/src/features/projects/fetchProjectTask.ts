import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
} from "@/shared/constants/kinds";
import { fetchAssignmentOperationEvents } from "./assignmentOperationFetch";
import { eventToProjectIssue, type ProjectIssue } from "./projectIssues.mjs";

type FetchEvents = (
  filter: Parameters<(typeof relayClient)["fetchEvents"]>[0],
) => Promise<RelayEvent[]>;

const STATUS_PAGE_LIMIT = 500;
/** Hard cap on status history read for one task (2,000 events). */
const STATUS_MAX_PAGES = 4;
const STATUS_KINDS = [
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
];

const HEX64_RE = /^[a-fA-F0-9]{64}$/;

/**
 * Canonical NIP-34 repository coordinate, or null when malformed. Only the
 * owner pubkey is case-insensitive hex; the `d` identifier is compared
 * exactly, so `Relay` and `relay` are different repositories.
 */
export function normalizeRepoAddress(address: string | null | undefined) {
  if (!address) return null;
  const first = address.indexOf(":");
  const second = address.indexOf(":", first + 1);
  if (first < 0 || second < 0) return null;
  const kind = address.slice(0, first);
  const owner = address.slice(first + 1, second);
  const dtag = address.slice(second + 1);
  if (kind !== "30617" || !HEX64_RE.test(owner) || dtag.length === 0) {
    return null;
  }
  return `30617:${owner.toLowerCase()}:${dtag}`;
}

/**
 * Read every status event for one task within an explicit cap. The relay
 * returns newest first; walking an inclusive `until` cursor with id dedupe
 * means a flood of untrusted statuses cannot push the latest authorized one
 * out of view. A history beyond the cap, or a single second denser than a
 * page, rejects instead of reducing over a partial set.
 */
async function fetchStatusEvents(
  issueId: string,
  fetchEvents: FetchEvents,
  signal?: AbortSignal,
): Promise<RelayEvent[]> {
  const byId = new Map<string, RelayEvent>();
  let until: number | undefined;
  for (let page = 0; page < STATUS_MAX_PAGES; page += 1) {
    signal?.throwIfAborted();
    const events = await fetchEvents({
      kinds: STATUS_KINDS,
      "#e": [issueId],
      limit: STATUS_PAGE_LIMIT,
      ...(until === undefined ? {} : { until }),
    });
    let added = 0;
    for (const event of events) {
      if (!byId.has(event.id)) {
        byId.set(event.id, event);
        added += 1;
      }
    }
    if (events.length < STATUS_PAGE_LIMIT) return [...byId.values()];
    if (added === 0) break;
    until = Math.min(...events.map((event) => event.created_at));
  }
  throw new Error("Task status history is too large to verify.");
}

/**
 * Load one task by its exact event id and reduce it with the same rules as
 * the project board. Returns null when the relay has no kind:1621 event with
 * that id or when the event belongs to a different repository than the link
 * named, so a forged or stale link can never borrow another task's state.
 *
 * Status and assignment histories are read completely; a failure or an
 * oversized history rejects rather than returning a task whose status or
 * assignees silently miss authorized events.
 */
export async function fetchProjectTask(
  issueId: string,
  repoAddress: string,
  fetchEvents: FetchEvents = relayClient.fetchEvents.bind(relayClient),
  signal?: AbortSignal,
): Promise<ProjectIssue | null> {
  const id = issueId.toLowerCase();
  const expectedRepo = normalizeRepoAddress(repoAddress);
  if (!expectedRepo) return null;
  const [issueEvents, statusEvents, assignmentEvents] = await Promise.all([
    fetchEvents({ kinds: [KIND_GIT_ISSUE], ids: [id], limit: 1 }),
    fetchStatusEvents(id, fetchEvents, signal),
    fetchAssignmentOperationEvents([id], fetchEvents, signal),
  ]);
  signal?.throwIfAborted();
  const issue = issueEvents.find(
    (event) => event.kind === KIND_GIT_ISSUE && event.id.toLowerCase() === id,
  );
  if (!issue) return null;
  const reduced = eventToProjectIssue(issue, statusEvents, assignmentEvents);
  if (normalizeRepoAddress(reduced.repoAddress) !== expectedRepo) return null;
  return reduced;
}
