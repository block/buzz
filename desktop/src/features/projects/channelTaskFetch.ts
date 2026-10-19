import { relayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import { getTag, projectIssueEventsToIssues } from "./projectIssues.mjs";
import type { ProjectIssue } from "./projectIssues.mjs";

type FetchEvents = (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
type EventPage = { events: RelayEvent[]; complete: boolean };

export type ChannelTaskResult = {
  issues: ProjectIssue[];
  rootsComplete: boolean;
  assignmentsComplete: boolean;
  statusesComplete: boolean;
};

function boundedTaskFetcher(
  fetchEvents: FetchEvents,
  signal?: AbortSignal,
): FetchEvents {
  let requestsRemaining = MAX_REQUESTS;
  return (filter) => {
    signal?.throwIfAborted();
    if (requestsRemaining <= 0) {
      throw new Error("Task history exceeded the read budget.");
    }
    requestsRemaining -= 1;
    return fetchEvents(filter);
  };
}

const PAGE_LIMIT = 500;
const MAX_PAGE_LIMIT = 1_000;
const MAX_PAGES = 10;
const MAX_TASKS = 500;
const ID_CHUNK_SIZE = 100;
const MAX_REQUESTS = 24;

async function readPages(
  filter: Omit<RelaySubscriptionFilter, "limit">,
  fetchEvents: FetchEvents,
  signal?: AbortSignal,
): Promise<EventPage> {
  const events = new Map<string, RelayEvent>();
  let until: number | undefined;
  let limit = PAGE_LIMIT;
  for (let pageNumber = 0; pageNumber < MAX_PAGES; pageNumber += 1) {
    signal?.throwIfAborted();
    const page = await fetchEvents({
      ...filter,
      limit,
      ...(until === undefined ? {} : { until }),
    });
    signal?.throwIfAborted();
    for (const event of page) events.set(event.id, event);
    if (page.length < limit)
      return { events: [...events.values()], complete: true };
    const oldest = Math.min(...page.map((event) => event.created_at));
    if (until === undefined || oldest < until) {
      until = oldest;
    } else if (limit < MAX_PAGE_LIMIT) {
      limit = MAX_PAGE_LIMIT;
    } else {
      break;
    }
  }
  return { events: [...events.values()], complete: false };
}

async function readTaskState(
  ids: string[],
  kinds: number[],
  fetchEvents: FetchEvents,
  signal?: AbortSignal,
): Promise<EventPage> {
  const events: RelayEvent[] = [];
  let complete = true;
  // Sequential, bounded batches avoid a request burst on opening a channel.
  for (let offset = 0; offset < ids.length; offset += ID_CHUNK_SIZE) {
    signal?.throwIfAborted();
    try {
      const page = await readPages(
        { kinds, "#e": ids.slice(offset, offset + ID_CHUNK_SIZE) },
        fetchEvents,
        signal,
      );
      events.push(...page.events);
      complete &&= page.complete;
    } catch {
      signal?.throwIfAborted();
      complete = false;
    }
  }
  return { events, complete };
}

/** Read channel-origin issues without trusting post-filtered, capped repository queries. */
export async function fetchChannelTasks(
  channelId: string,
  repoAddresses: readonly string[],
  signal?: AbortSignal,
  fetchEvents: FetchEvents = relayClient.fetchEvents.bind(relayClient),
): Promise<ChannelTaskResult> {
  if (repoAddresses.length === 0) {
    return {
      issues: [],
      rootsComplete: true,
      assignmentsComplete: true,
      statusesComplete: true,
    };
  }
  const boundedFetch = boundedTaskFetcher(fetchEvents, signal);
  // NIP-34 events are stored globally even when they carry an origin h tag.
  // Neither #h nor #a can safely page those roots. Scan only SQL-pushed kinds
  // and until, then match the author-claimed origin and visible repositories.
  const roots = await readPages(
    { kinds: [KIND_GIT_ISSUE] },
    boundedFetch,
    signal,
  );
  const addresses = new Set(repoAddresses);
  const matching = roots.events.filter(
    (event) =>
      event.kind === KIND_GIT_ISSUE &&
      getTag(event, "h") === channelId &&
      addresses.has(getTag(event, "a") ?? ""),
  );
  const issueEvents = matching.slice(0, MAX_TASKS);
  return readTaskDetails(
    issueEvents,
    roots.complete && matching.length <= MAX_TASKS,
    boundedFetch,
    signal,
  );
}

async function readTaskDetails(
  issueEvents: RelayEvent[],
  rootsComplete: boolean,
  fetchEvents: FetchEvents,
  signal?: AbortSignal,
): Promise<ChannelTaskResult> {
  const ids = issueEvents.map((event) => event.id);
  const [comments, statuses] = await Promise.all([
    readTaskState(ids, [KIND_TEXT_NOTE], fetchEvents, signal),
    readTaskState(
      ids,
      [
        KIND_GIT_STATUS_OPEN,
        KIND_GIT_STATUS_MERGED,
        KIND_GIT_STATUS_CLOSED,
        KIND_GIT_STATUS_DRAFT,
      ],
      fetchEvents,
      signal,
    ),
  ]);
  signal?.throwIfAborted();
  return {
    issues: projectIssueEventsToIssues(
      issueEvents,
      statuses.events,
      comments.events,
    ),
    rootsComplete,
    assignmentsComplete: comments.complete,
    statusesComplete: statuses.complete,
  };
}

/** Resolve one selected task independently of the bounded project list. */
export async function fetchProjectTask(
  repoAddress: string,
  issueId: string,
  signal?: AbortSignal,
  fetchEvents: FetchEvents = relayClient.fetchEvents.bind(relayClient),
): Promise<ProjectIssue[]> {
  if (!/^[a-f0-9]{64}$/i.test(issueId))
    throw new Error("Invalid task identifier.");
  const normalizedId = issueId.toLowerCase();
  const boundedFetch = boundedTaskFetcher(fetchEvents, signal);
  const roots = await boundedFetch({
    kinds: [KIND_GIT_ISSUE],
    ids: [normalizedId],
    limit: 1,
  });
  signal?.throwIfAborted();
  const root = roots.find(
    (event) =>
      event.id === normalizedId &&
      event.kind === KIND_GIT_ISSUE &&
      getTag(event, "a") === repoAddress,
  );
  if (!root)
    throw new Error(
      "This task is unavailable. Return to the project task list.",
    );
  const result = await readTaskDetails([root], true, boundedFetch, signal);
  if (!result.assignmentsComplete || !result.statusesComplete) {
    throw new Error(
      "Could not load this task's current status and assignments. Try again.",
    );
  }
  return result.issues;
}
