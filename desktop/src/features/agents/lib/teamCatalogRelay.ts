import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_TEAM_CATALOG } from "@/shared/constants/kinds";
import { isSafeDisplayText } from "@/shared/lib/safeDisplayText";

const TEAM_CATALOG_SCHEMA_VERSION = 1;
const CATALOG_PAGE_SIZE = 500;
const MAX_CATALOG_PAGES = 40;
const MAX_CATALOG_EVENTS = CATALOG_PAGE_SIZE * MAX_CATALOG_PAGES;
const MAX_CATALOG_VALIDATION_BYTES = 4 * 1024 * 1024;
const MAX_OWNER_VALIDATION_BYTES = 512 * 1024;
const MAX_EVENT_CONTENT_BYTES = 192 * 1024;
const MAX_MEMBERS = 64;
const MAX_NAME_BYTES = 256;
const MAX_MEMBER_KEY_BYTES = 128;
const MAX_TEXT_BYTES = 4 * 1024;
const MAX_SYSTEM_PROMPT_BYTES = 16 * 1024;
const MAX_IDENTIFIER_BYTES = 256;
const MAX_NAME_POOL_ENTRIES = 64;
const MAX_EVENT_TAGS = 128;
const MAX_EVENT_TAG_FIELDS = 8;
const MAX_EVENT_TAG_FIELD_BYTES = 256;
const NOSTR_HEX = /^[0-9a-f]{64}$/u;

export type TeamCatalogPublication = {
  eventId: string;
  ownerPubkey: string;
  teamDTag: string;
  name: string;
  memberCount: number;
  /** Opaque member keys used only for relay-aware team resolution. */
  memberKeys: string[];
};

type JsonObject = Record<string, unknown>;
type CatalogMember = JsonObject & {
  member_key: string;
  display_name: string;
};
type SafeCatalogEvent = {
  id: string;
  ownerPubkey: string;
  teamDTag: string;
  createdAt: number;
  content: string;
  shared: boolean;
  coordinate: string;
};
type CatalogHead = {
  createdAt: number;
  eventId: string;
  publication: TeamCatalogPublication | null;
};
type CatalogAccumulator = {
  heads: Map<string, CatalogHead>;
  validationBytes: number;
  validationBytesByOwner: Map<string, number>;
};

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

function withinBytes(value: string, maximum: number): boolean {
  return utf8ByteLength(value) <= maximum;
}

function safeDisplayText(value: string, maximum: number): boolean {
  return isSafeDisplayText(value, maximum);
}

function scanContent(value: string, maximum: number) {
  const bytes = utf8ByteLength(value);
  return bytes <= maximum ? bytes : null;
}

function validDTag(value: string): boolean {
  return (
    value.trim().length > 0 &&
    Array.from(value).length <= 64 &&
    !/[\s\p{C}]/u.test(value)
  );
}

function readCatalogTags(value: unknown): {
  teamDTag: string;
  shared: boolean;
} | null {
  if (!Array.isArray(value) || value.length > MAX_EVENT_TAGS) return null;

  let dTagCount = 0;
  let teamDTag: string | null = null;
  let sharedTagCount = 0;
  let exactSharedTag = false;

  for (const tag of value) {
    if (
      !Array.isArray(tag) ||
      tag.length === 0 ||
      tag.length > MAX_EVENT_TAG_FIELDS ||
      tag.some(
        (field) =>
          typeof field !== "string" ||
          !withinBytes(field, MAX_EVENT_TAG_FIELD_BYTES),
      )
    ) {
      return null;
    }

    if (tag[0] === "d") {
      dTagCount += 1;
      if (tag.length === 2 && typeof tag[1] === "string") {
        teamDTag = tag[1];
      }
    }
    if (tag[0] === "shared") {
      sharedTagCount += 1;
      exactSharedTag = tag.length === 2 && tag[1] === "true";
    }
  }

  if (
    dTagCount !== 1 ||
    teamDTag === null ||
    !withinBytes(teamDTag, MAX_EVENT_TAG_FIELD_BYTES) ||
    !validDTag(teamDTag)
  ) {
    return null;
  }

  return {
    teamDTag,
    shared: sharedTagCount === 1 && exactSharedTag,
  };
}

function assessCatalogEvent(value: unknown): {
  event: SafeCatalogEvent | null;
  unsafeCandidate: boolean;
} {
  try {
    if (!isObject(value) || value.kind !== KIND_TEAM_CATALOG) {
      return { event: null, unsafeCandidate: false };
    }

    if (
      typeof value.id !== "string" ||
      !NOSTR_HEX.test(value.id) ||
      typeof value.pubkey !== "string" ||
      !NOSTR_HEX.test(value.pubkey) ||
      typeof value.created_at !== "number" ||
      !Number.isSafeInteger(value.created_at) ||
      value.created_at < 0 ||
      typeof value.content !== "string"
    ) {
      return { event: null, unsafeCandidate: true };
    }

    const tags = readCatalogTags(value.tags);
    if (!tags) return { event: null, unsafeCandidate: true };

    return {
      event: {
        id: value.id,
        ownerPubkey: value.pubkey,
        teamDTag: tags.teamDTag,
        createdAt: value.created_at,
        content: value.content,
        shared: tags.shared,
        coordinate: `${value.pubkey}:${tags.teamDTag}`,
      },
      unsafeCandidate: false,
    };
  } catch {
    return { event: null, unsafeCandidate: true };
  }
}

function optionalBoundedString(value: unknown, maximum: number): boolean {
  return (
    value === undefined ||
    value === null ||
    (typeof value === "string" && withinBytes(value, maximum))
  );
}

function validCatalogMember(value: unknown): value is CatalogMember {
  if (!isObject(value)) return false;
  if (
    typeof value.member_key !== "string" ||
    value.member_key.trim().length === 0 ||
    !withinBytes(value.member_key, MAX_MEMBER_KEY_BYTES) ||
    typeof value.display_name !== "string" ||
    value.display_name.trim().length === 0 ||
    !safeDisplayText(value.display_name, MAX_NAME_BYTES)
  ) {
    return false;
  }
  if (
    !optionalBoundedString(value.system_prompt, MAX_SYSTEM_PROMPT_BYTES) ||
    !optionalBoundedString(value.avatar_url, MAX_TEXT_BYTES * 8)
  ) {
    return false;
  }
  for (const field of ["runtime", "model", "provider"] as const) {
    const candidate = value[field];
    if (
      candidate !== undefined &&
      candidate !== null &&
      (typeof candidate !== "string" ||
        candidate.trim().length === 0 ||
        !withinBytes(candidate, MAX_IDENTIFIER_BYTES))
    ) {
      return false;
    }
  }
  if (value.name_pool !== undefined) {
    if (
      !Array.isArray(value.name_pool) ||
      value.name_pool.length > MAX_NAME_POOL_ENTRIES ||
      value.name_pool.some(
        (name) =>
          typeof name !== "string" ||
          name.trim().length === 0 ||
          !withinBytes(name, MAX_NAME_BYTES),
      )
    ) {
      return false;
    }
  }
  if (
    value.respond_to !== undefined &&
    value.respond_to !== null &&
    value.respond_to !== "owner-only" &&
    value.respond_to !== "allowlist" &&
    value.respond_to !== "anyone"
  ) {
    return false;
  }
  return (
    value.parallelism === undefined ||
    value.parallelism === null ||
    (typeof value.parallelism === "number" &&
      Number.isInteger(value.parallelism) &&
      value.parallelism >= 1 &&
      value.parallelism <= 32)
  );
}

function parseCatalogContent(
  content: string,
  contentBytes: number,
): { name: string; memberKeys: string[] } | null {
  if (contentBytes > MAX_EVENT_CONTENT_BYTES) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch {
    return null;
  }
  if (
    !isObject(parsed) ||
    parsed.v !== TEAM_CATALOG_SCHEMA_VERSION ||
    typeof parsed.name !== "string" ||
    parsed.name.trim().length === 0 ||
    !safeDisplayText(parsed.name, MAX_NAME_BYTES) ||
    !Array.isArray(parsed.members) ||
    parsed.members.length > MAX_MEMBERS ||
    !optionalBoundedString(parsed.description, MAX_TEXT_BYTES) ||
    !optionalBoundedString(parsed.instructions, MAX_TEXT_BYTES)
  ) {
    return null;
  }

  const memberKeys = new Set<string>();
  for (const member of parsed.members) {
    if (!validCatalogMember(member) || memberKeys.has(member.member_key)) {
      return null;
    }
    memberKeys.add(member.member_key);
  }

  return { name: parsed.name, memberKeys: [...memberKeys] };
}

function newerThan(event: SafeCatalogEvent, head: CatalogHead): boolean {
  return (
    event.createdAt > head.createdAt ||
    (event.createdAt === head.createdAt &&
      event.id.localeCompare(head.eventId) < 0)
  );
}

function considerEvent(
  accumulator: CatalogAccumulator,
  event: SafeCatalogEvent,
): void {
  const previous = accumulator.heads.get(event.coordinate);
  if (previous && !newerThan(event, previous)) return;

  let publication: TeamCatalogPublication | null = null;
  if (event.shared) {
    const ownerValidationBytes =
      accumulator.validationBytesByOwner.get(event.ownerPubkey) ?? 0;
    const remaining = Math.min(
      MAX_CATALOG_VALIDATION_BYTES - accumulator.validationBytes,
      MAX_OWNER_VALIDATION_BYTES - ownerValidationBytes,
    );
    if (remaining > 0) {
      const contentBytes = scanContent(
        event.content,
        Math.min(MAX_EVENT_CONTENT_BYTES, remaining),
      );
      accumulator.validationBytes += contentBytes ?? remaining;
      accumulator.validationBytesByOwner.set(
        event.ownerPubkey,
        ownerValidationBytes + (contentBytes ?? remaining),
      );
      if (contentBytes !== null) {
        const content = parseCatalogContent(event.content, contentBytes);
        if (content) {
          publication = {
            eventId: event.id,
            ownerPubkey: event.ownerPubkey,
            teamDTag: event.teamDTag,
            name: content.name,
            memberCount: content.memberKeys.length,
            memberKeys: content.memberKeys,
          };
        }
      }
    }
  }

  // Claim the coordinate even when the current head is unshared or malformed.
  accumulator.heads.set(event.coordinate, {
    createdAt: event.createdAt,
    eventId: event.id,
    publication,
  });
}

function sortedEvents(events: SafeCatalogEvent[]): SafeCatalogEvent[] {
  return events.sort(
    (left, right) =>
      right.createdAt - left.createdAt || left.id.localeCompare(right.id),
  );
}

function safeEvents(values: Iterable<unknown>, maximum: number) {
  const events: SafeCatalogEvent[] = [];
  const seenIds = new Set<string>();
  let inspected = 0;
  for (const value of values) {
    if (inspected >= maximum) {
      return { events, truncated: true, unsafeCandidate: false };
    }
    inspected += 1;
    const assessment = assessCatalogEvent(value);
    if (assessment.unsafeCandidate) {
      return { events, truncated: false, unsafeCandidate: true };
    }
    if (assessment.event && !seenIds.has(assessment.event.id)) {
      seenIds.add(assessment.event.id);
      events.push(assessment.event);
    }
  }
  return { events, truncated: false, unsafeCandidate: false };
}

function publicationsFromHeads(
  heads: ReadonlyMap<string, CatalogHead>,
): TeamCatalogPublication[] {
  return [...heads.values()]
    .flatMap((head) => (head.publication ? [head.publication] : []))
    .sort((left, right) => left.name.localeCompare(right.name));
}

function suppressAll(accumulator: CatalogAccumulator): void {
  for (const [coordinate, head] of accumulator.heads) {
    accumulator.heads.set(coordinate, { ...head, publication: null });
  }
}

function suppressAtTimestamp(
  accumulator: CatalogAccumulator,
  timestamp: number,
): void {
  for (const [coordinate, head] of accumulator.heads) {
    if (head.createdAt === timestamp) {
      accumulator.heads.set(coordinate, { ...head, publication: null });
    }
  }
}

/** Only an exact `['shared', 'true']` declaration opts a head into discovery. */
export function teamEventIsShared(event: RelayEvent): boolean {
  return assessCatalogEvent(event).event?.shared ?? false;
}

/** Resolve complete, safe public projections using replaceable-head semantics. */
export function teamCatalogPublicationsFromEvents(
  events: readonly RelayEvent[],
): TeamCatalogPublication[] {
  const accumulator: CatalogAccumulator = {
    heads: new Map(),
    validationBytes: 0,
    validationBytesByOwner: new Map(),
  };
  const safe = safeEvents(events, MAX_CATALOG_EVENTS);
  for (const event of sortedEvents(safe.events)) {
    considerEvent(accumulator, event);
  }
  if (safe.truncated || safe.unsafeCandidate) suppressAll(accumulator);
  return publicationsFromHeads(accumulator.heads);
}

function catalogPage(value: unknown) {
  if (!Array.isArray(value)) {
    return { events: [], full: false, unsafeCandidate: true };
  }
  const rawEvents = value.slice(0, CATALOG_PAGE_SIZE);
  const safe = safeEvents(rawEvents, CATALOG_PAGE_SIZE);
  return {
    events: sortedEvents(safe.events),
    full: rawEvents.length === CATALOG_PAGE_SIZE,
    unsafeCandidate: value.length > CATALOG_PAGE_SIZE || safe.unsafeCandidate,
  };
}

/** Read all bounded catalog pages and deduplicate inclusive cursor repeats. */
export async function fetchTeamCatalogPublications(): Promise<
  TeamCatalogPublication[]
> {
  const accumulator: CatalogAccumulator = {
    heads: new Map(),
    validationBytes: 0,
    validationBytesByOwner: new Map(),
  };
  const seenEventIds = new Set<string>();
  let until: number | undefined;

  for (let page = 0; page < MAX_CATALOG_PAGES; page += 1) {
    const response = await relayClient.fetchEvents({
      kinds: [KIND_TEAM_CATALOG],
      limit: CATALOG_PAGE_SIZE,
      ...(until === undefined ? {} : { until }),
    });
    const catalog = catalogPage(response);
    if (catalog.unsafeCandidate) return [];

    const sizeBefore = seenEventIds.size;
    let oldestCreatedAt = Number.POSITIVE_INFINITY;
    for (const event of catalog.events) {
      oldestCreatedAt = Math.min(oldestCreatedAt, event.createdAt);
      if (seenEventIds.has(event.id)) continue;
      seenEventIds.add(event.id);
      considerEvent(accumulator, event);
    }

    if (!catalog.full) break;
    if (!Number.isFinite(oldestCreatedAt)) {
      suppressAll(accumulator);
      break;
    }
    if (seenEventIds.size === sizeBefore || page === MAX_CATALOG_PAGES - 1) {
      suppressAtTimestamp(accumulator, oldestCreatedAt);
      break;
    }
    until = oldestCreatedAt;
  }

  return publicationsFromHeads(accumulator.heads);
}
