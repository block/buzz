import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import { buildProjectReadModels, type Project } from "./projectModels";

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
      // Invariant violation: the relay has more events sharing this exact
      // second than the page limit. Enumeration is statically uncompletable
      // at the current page size. Rather than present a silently truncated
      // collection, we hard-error. If this surfaces in production, the fix is
      // either a larger pageSize constant or a relay-side deduplication pass.
      // TODO: add a telemetry event here so pathological relay states are
      // diagnosable before they reach users.
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

/**
 * Core fetch-and-build logic for `fetchProjects`, extracted for testability.
 *
 * Accepts an injectable `fetchExhaustively` so unit tests can stub individual
 * kind enumerations (including injecting a rejection for kind:5 tombstones) without
 * pulling in the Tauri relay client.
 *
 * Fail-closed: if the kind:5 tombstone enumeration rejects, throws rather than
 * returning an empty deletion set that would resurrect every deleted head.
 */
export async function buildProjectsFromFetcher(
  fetchExhaustively: (kinds: number[]) => Promise<RelayEvent[]>,
  options: {
    relayOrigin?: string | null;
    hiddenAddresses?: ReadonlySet<string>;
  } = {},
): Promise<Project[]> {
  const [projectEvents, repositoryEvents, tombstoneResult] = await Promise.all([
    fetchExhaustively([KIND_PROJECT_ANNOUNCEMENT]),
    fetchExhaustively([KIND_REPO_ANNOUNCEMENT]),
    fetchExhaustively([KIND_DELETION]).then(
      (events) => ({ ok: true as const, events }),
      (error: unknown) => ({
        ok: false as const,
        message: error instanceof Error ? error.message : "Unknown error",
      }),
    ),
  ]);

  if (!tombstoneResult.ok) {
    throw new Error(
      `Could not fetch project deletion records: ${tombstoneResult.message} — refresh to retry.`,
    );
  }

  return buildProjectReadModels({
    projectEvents,
    repositoryEvents,
    deletionEvents: tombstoneResult.events,
    relayOrigin: options.relayOrigin ?? null,
    hiddenAddresses: options.hiddenAddresses ?? new Set(),
  }).sort((a, b) => b.createdAt - a.createdAt);
}
