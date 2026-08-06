import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import { buildProjectReadModels, type Project } from "./projectModels";

const PROJECT_ENUMERATION_PAGE_SIZE = 500;

// Relays commonly cap filter tag-value lists; chunk `#a` scoping well below
// any such cap.
const TOMBSTONE_COORDINATE_CHUNK_SIZE = 100;

/** Additional server-side scoping merged into every enumeration page. */
export type ProjectEventExtraFilter = {
  "#a"?: string[];
};

type ProjectEventFilter = ProjectEventExtraFilter & {
  kinds: number[];
  limit: number;
  since?: number;
  until?: number;
};

export type FetchProjectEventsExhaustively = (
  kinds: number[],
  extraFilter?: ProjectEventExtraFilter,
) => Promise<RelayEvent[]>;

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
  extraFilter?: ProjectEventExtraFilter,
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
      ...extraFilter,
      kinds,
      limit: pageSize,
      ...(until === undefined ? {} : { until }),
    });
    for (const event of page) eventsById.set(event.id, event);
    if (page.length < pageSize) return [...eventsById.values()];

    const oldest = Math.min(...page.map((event) => event.created_at));
    const boundary = await fetchPage({
      ...extraFilter,
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
  extraFilter?: ProjectEventExtraFilter,
  pageSize = PROJECT_ENUMERATION_PAGE_SIZE,
): Promise<RelayEvent[]> {
  return enumerateProjectEvents(
    (filter) => relayClient.fetchEvents(filter),
    kinds,
    pageSize,
    extraFilter,
  );
}

/** `kind:owner:dtag` coordinate for an addressable announcement event. */
function eventCoordinate(event: RelayEvent): string | null {
  const dtag = event.tags.find((tag) => tag[0] === "d")?.[1];
  if (typeof dtag !== "string" || dtag.length === 0) return null;
  return `${event.kind}:${event.pubkey.toLowerCase()}:${dtag}`;
}

/**
 * Fetches the NIP-09 kind:5 tombstones relevant to the given announcement
 * events, scoped server-side with `#a` filters on the announcements' own
 * coordinates. Kind:5 is the app-wide deletion kind (every deleted chat
 * message is one), so enumerating it unscoped crawls the entire community's
 * deletion history — minutes on a large relay. Only tombstones addressing a
 * currently visible project/repo coordinate can affect the read models, so
 * scoping is semantically equivalent (see `buildDeletionThresholds`).
 */
async function fetchScopedDeletionEvents(
  fetchExhaustively: FetchProjectEventsExhaustively,
  announcementEvents: RelayEvent[],
): Promise<RelayEvent[]> {
  const coordinates = [
    ...new Set(
      announcementEvents.flatMap((event) => {
        const coordinate = eventCoordinate(event);
        return coordinate ? [coordinate] : [];
      }),
    ),
  ];
  if (coordinates.length === 0) return [];

  const chunks: string[][] = [];
  for (
    let index = 0;
    index < coordinates.length;
    index += TOMBSTONE_COORDINATE_CHUNK_SIZE
  ) {
    chunks.push(
      coordinates.slice(index, index + TOMBSTONE_COORDINATE_CHUNK_SIZE),
    );
  }

  const pages = await Promise.all(
    chunks.map((chunk) => fetchExhaustively([KIND_DELETION], { "#a": chunk })),
  );
  return pages.flat();
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
  fetchExhaustively: FetchProjectEventsExhaustively,
  options: {
    relayOrigin?: string | null;
    hiddenAddresses?: ReadonlySet<string>;
  } = {},
): Promise<Project[]> {
  const [projectEvents, repositoryEvents] = await Promise.all([
    fetchExhaustively([KIND_PROJECT_ANNOUNCEMENT]),
    fetchExhaustively([KIND_REPO_ANNOUNCEMENT]),
  ]);

  // Tombstones are fetched second (not in parallel) because the `#a` scoping
  // needs the announcement coordinates; both announcement kinds are small,
  // so this costs one extra round trip, not a full crawl.
  const tombstoneResult = await fetchScopedDeletionEvents(fetchExhaustively, [
    ...projectEvents,
    ...repositoryEvents,
  ]).then(
    (events) => ({ ok: true as const, events }),
    (error: unknown) => ({
      ok: false as const,
      message: error instanceof Error ? error.message : "Unknown error",
    }),
  );

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
