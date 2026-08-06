import {
  fetchCatalogEvents,
  safeCatalogAvatarUrl,
  sharedCatalogHeads,
} from "@/features/agents/lib/catalogRelay";
import type { AgentTeam, RelayEvent } from "@/shared/api/types";
import { KIND_TEAM_CATALOG } from "@/shared/constants/kinds";

/**
 * Read the kind:30178 team catalog.
 *
 * The projection is self-contained by design: every member's safe definition
 * is embedded, so this module renders a published team without resolving
 * anything in the publisher's namespace. `member_key` is deliberately treated
 * as an opaque label here and never as a kind:30175 coordinate — the publisher
 * may never have shared that member individually.
 *
 * Adding is NOT done from this data. The frontend passes only the coordinate
 * to `add_team_from_catalog`, which re-fetches and re-verifies the head
 * backend-side; what is parsed here is for display and for deciding whether
 * "Add" is offered.
 */

/** Schema version this client understands. Must match the backend's. */
const TEAM_CATALOG_SCHEMA_VERSION = 1;

// ── v1 size bounds — must stay in sync with team_catalog.rs ──────────────
const MAX_MEMBERS = 64;
const MAX_NAME_BYTES = 256;
const MAX_TEXT_BYTES = 4 * 1024;
const MAX_INSTRUCTIONS_BYTES = 16 * 1024;
const MAX_SYSTEM_PROMPT_BYTES = 16 * 1024;
const MAX_AVATAR_URL_BYTES = 32 * 1024;
const MAX_NAME_POOL_ENTRIES = 64;
const MAX_TOTAL_BYTES = 192 * 1024;
const MAX_MEMBER_KEY_BYTES = 128;
const MAX_IDENTIFIER_BYTES = 256;
const MAX_BUILTIN_SLUG_BYTES = 128;

/** Recognized respond_to wire values. Must stay in sync with RespondTo::parse_wire. */
const RESPOND_TO_VALUES = new Set(["owner-only", "allowlist", "anyone"]);

function byteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

function withinBytes(value: string, max: number): boolean {
  return byteLength(value) <= max;
}

export type CatalogTeamMember = {
  memberKey: string;
  displayName: string;
  systemPrompt: string;
  avatarUrl: string | null;
  runtime: string | null;
  model: string | null;
  provider: string | null;
};

export type CatalogTeam = {
  /** The head event this projection was built from. Passed to the backend so
   *  it can reject an add whose head moved since the dialog opened. */
  eventId: string;
  ownerPubkey: string;
  teamDTag: string;
  isOwn: boolean;
  name: string;
  description: string | null;
  instructions: string | null;
  members: CatalogTeamMember[];
  /**
   * Number of members that failed v1 validation. When non-zero the team is
   * displayed with a diagnostic and the Add button is disabled — a team with
   * invalid members cannot be safely adopted.
   */
  invalidMemberCount: number;
  /** The local team already copied from this publication, if any. */
  localTeam: AgentTeam | null;
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

/**
 * Validate one member against the v1 contract. Returns true when the member
 * passes all checks, false when any bound is exceeded or a recognized value
 * is unrecognized. Mirrors validate_member in team_catalog.rs.
 */
function memberPassesV1(value: unknown): boolean {
  if (!isObject(value)) return false;

  // Required: non-empty member_key within bounds
  if (
    typeof value.member_key !== "string" ||
    value.member_key.trim().length === 0 ||
    !withinBytes(value.member_key, MAX_MEMBER_KEY_BYTES)
  ) {
    return false;
  }

  // Required: non-empty display_name within bounds
  if (
    typeof value.display_name !== "string" ||
    value.display_name.trim().length === 0 ||
    !withinBytes(value.display_name, MAX_NAME_BYTES)
  ) {
    return false;
  }

  // Optional bounded fields — reject any present wrong-typed value (I8).
  // A present non-string must fail validation, not be ignored.
  if (
    value.system_prompt !== undefined &&
    value.system_prompt !== null &&
    (typeof value.system_prompt !== "string" ||
      !withinBytes(value.system_prompt, MAX_SYSTEM_PROMPT_BYTES))
  ) {
    return false;
  }
  if (
    value.avatar_url !== undefined &&
    value.avatar_url !== null &&
    (typeof value.avatar_url !== "string" ||
      !withinBytes(value.avatar_url, MAX_AVATAR_URL_BYTES) ||
      safeCatalogAvatarUrl(value.avatar_url) === null)
  ) {
    return false;
  }
  for (const field of ["runtime", "model", "provider"] as const) {
    const v = value[field];
    if (v !== undefined && v !== null) {
      if (
        typeof v !== "string" ||
        v.trim().length === 0 ||
        !withinBytes(v, MAX_IDENTIFIER_BYTES)
      ) {
        return false;
      }
    }
  }

  // name_pool: must be an array when present; non-array fails (I8).
  // null is NOT treated as absent — Rust's Vec<String> rejects explicit null.
  if (value.name_pool !== undefined) {
    if (value.name_pool === null || !Array.isArray(value.name_pool)) {
      return false;
    }
  }
  // name_pool: entry count and per-entry bounds
  if (Array.isArray(value.name_pool)) {
    if (value.name_pool.length > MAX_NAME_POOL_ENTRIES) return false;
    for (const entry of value.name_pool) {
      if (
        typeof entry !== "string" ||
        entry.trim().length === 0 ||
        !withinBytes(entry, MAX_NAME_BYTES)
      ) {
        return false;
      }
    }
  }

  // respond_to: recognized wire values only
  if (
    value.respond_to !== undefined &&
    value.respond_to !== null &&
    (typeof value.respond_to !== "string" ||
      !RESPOND_TO_VALUES.has(value.respond_to))
  ) {
    return false;
  }

  // parallelism: 1–32 when present
  if (value.parallelism !== undefined && value.parallelism !== null) {
    if (
      typeof value.parallelism !== "number" ||
      !Number.isInteger(value.parallelism) ||
      value.parallelism < 1 ||
      value.parallelism > 32
    ) {
      return false;
    }
  }

  // Built-in reuse hint: either both present and valid, or both absent.
  // A present-but-wrong-type value must fail (not be treated as absent).
  const slugPresent =
    value.builtin_slug !== undefined && value.builtin_slug !== null;
  const hashPresent =
    value.projection_hash !== undefined && value.projection_hash !== null;
  const hasSlug =
    typeof value.builtin_slug === "string" && value.builtin_slug.length > 0;
  const hasHash =
    typeof value.projection_hash === "string" &&
    value.projection_hash.length > 0;
  // Reject if present but wrong type (e.g., builtin_slug: 42 or projection_hash: {})
  if (slugPresent && !hasSlug) return false;
  if (hashPresent && !hasHash) return false;
  if (hasSlug !== hasHash) return false;
  if (hasSlug) {
    if (!withinBytes(value.builtin_slug as string, MAX_BUILTIN_SLUG_BYTES)) {
      return false;
    }
    // projection_hash must be a 64-char hex digest (case-insensitive, aligning
    // with the Rust validator which uses is_ascii_hexdigit() — I8).
    if (!/^[0-9a-fA-F]{64}$/.test(value.projection_hash as string)) {
      return false;
    }
  }

  return true;
}

function parseMember(value: unknown): CatalogTeamMember | null {
  if (!isObject(value)) return null;
  // Display-layer parsing: only needs the fields the UI renders.
  // Full schema validation is done by memberPassesV1 and by the backend
  // re-fetch on add.
  if (
    typeof value.member_key !== "string" ||
    value.member_key.length === 0 ||
    typeof value.display_name !== "string" ||
    value.display_name.trim().length === 0
  ) {
    return null;
  }
  return {
    memberKey: value.member_key,
    displayName: value.display_name,
    systemPrompt:
      typeof value.system_prompt === "string" ? value.system_prompt : "",
    avatarUrl: safeCatalogAvatarUrl(value.avatar_url),
    runtime: optionalString(value.runtime),
    model: optionalString(value.model),
    provider: optionalString(value.provider),
  };
}

/**
 * Parse a 30178 content body, rejecting an unrecognized schema version.
 *
 * Returns `null` when the body cannot be parsed at all (wrong version,
 * malformed JSON, missing required top-level fields, member-count cap
 * exceeded, or total-size cap exceeded). Otherwise returns the parsed
 * name/description/instructions/members and an `invalidMemberCount` for
 * members that fail the v1 contract. A non-zero count means the team should
 * be shown with a diagnostic and Add should be disabled.
 *
 * Version dispatch happens before field access, mirroring the backend: a
 * future `v: 2` body may legally reshape any field, so rendering whatever
 * happens to parse as `v: 1` would present a corrupted team as a valid one.
 */
export function parseTeamCatalogContent(
  event: RelayEvent,
): Pick<
  CatalogTeam,
  "name" | "description" | "instructions" | "members" | "invalidMemberCount"
> | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (
    !isObject(parsed) ||
    parsed.v !== TEAM_CATALOG_SCHEMA_VERSION ||
    typeof parsed.name !== "string" ||
    parsed.name.trim().length === 0 ||
    !withinBytes(parsed.name, MAX_NAME_BYTES) ||
    !Array.isArray(parsed.members)
  ) {
    return null;
  }

  // Team-level text bounds — reject present wrong-typed values (I8).
  if (
    parsed.description !== undefined &&
    parsed.description !== null &&
    (typeof parsed.description !== "string" ||
      !withinBytes(parsed.description, MAX_TEXT_BYTES))
  ) {
    return null;
  }
  if (
    parsed.instructions !== undefined &&
    parsed.instructions !== null &&
    (typeof parsed.instructions !== "string" ||
      !withinBytes(parsed.instructions, MAX_INSTRUCTIONS_BYTES))
  ) {
    return null;
  }

  // Member count cap: a body that exceeds this cannot be adopted anyway
  if (parsed.members.length > MAX_MEMBERS) {
    return null;
  }

  // Total-size cap: same reasoning
  if (byteLength(event.content) > MAX_TOTAL_BYTES) {
    return null;
  }

  // Parse display-layer fields; track members that fail v1 validation
  const members: CatalogTeamMember[] = [];
  let invalidMemberCount = 0;
  const seenMemberKeys = new Set<string>();
  for (const candidate of parsed.members) {
    // Check for duplicate member_key before other validation — a duplicate
    // is always invalid regardless of the member's other fields (I8).
    if (
      typeof (candidate as Record<string, unknown>)?.member_key === "string" &&
      seenMemberKeys.has(
        (candidate as Record<string, unknown>).member_key as string,
      )
    ) {
      invalidMemberCount++;
      continue;
    }
    if (!memberPassesV1(candidate)) {
      invalidMemberCount++;
      continue; // include the invalid count but still render the valid members
    }
    const member = parseMember(candidate);
    if (!member) {
      // parseMember is a subset of memberPassesV1; this branch is unreachable
      // for a candidate that passed v1, but guard defensively.
      invalidMemberCount++;
      continue;
    }
    seenMemberKeys.add(member.memberKey);
    members.push(member);
  }

  return {
    name: parsed.name,
    description: optionalString(parsed.description),
    instructions: optionalString(parsed.instructions),
    members,
    invalidMemberCount,
  };
}

export type TeamCatalogPublication = Omit<CatalogTeam, "isOwn" | "localTeam">;

/**
 * Project the shared kind:30178 heads onto team publications.
 *
 * A head whose content does not parse is dropped, not retried against an older
 * event: the coordinate is already claimed, so falling back would resurrect a
 * superseded definition.
 */
export function teamCatalogPublicationsFromEvents(
  events: readonly RelayEvent[],
): TeamCatalogPublication[] {
  const publications: TeamCatalogPublication[] = [];

  for (const head of sharedCatalogHeads(events, KIND_TEAM_CATALOG)) {
    const content = parseTeamCatalogContent(head.event);
    if (!content) continue;
    publications.push({
      eventId: head.event.id,
      ownerPubkey: head.ownerPubkey,
      teamDTag: head.dTag,
      ...content,
    });
  }

  return publications;
}

/** Read every shared team event, page by page. */
export async function fetchTeamCatalogPublications(): Promise<
  TeamCatalogPublication[]
> {
  return teamCatalogPublicationsFromEvents(
    await fetchCatalogEvents(KIND_TEAM_CATALOG),
  );
}

/**
 * The local team backing a catalog entry, if the user already has it.
 *
 * An own publication is found by id — its `d`-tag *is* the local team id. A
 * copy of another owner's entry carries a fresh local id instead, so the only
 * link back is the `catalogSource` coordinate stored on the copy. Matching on
 * that coordinate is what stops the catalog from offering "Add" for an entry
 * the user already added, which would mint a second copy.
 */
export function findLocalTeamForCatalogEntry(
  localTeams: readonly AgentTeam[],
  publication: TeamCatalogPublication,
  isOwn: boolean,
): AgentTeam | null {
  if (isOwn) {
    return localTeams.find((team) => team.id === publication.teamDTag) ?? null;
  }
  return (
    localTeams.find(
      (team) =>
        team.catalogSource?.ownerPubkey === publication.ownerPubkey &&
        team.catalogSource?.teamDTag === publication.teamDTag,
    ) ?? null
  );
}

export function catalogTeamsFromPublications(
  publications: readonly TeamCatalogPublication[],
  localTeams: readonly AgentTeam[],
  currentPubkey: string | null | undefined,
): CatalogTeam[] {
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase() ?? null;

  return publications
    .map((publication) => {
      const isOwn = publication.ownerPubkey === normalizedCurrentPubkey;
      return {
        ...publication,
        isOwn,
        localTeam: findLocalTeamForCatalogEntry(localTeams, publication, isOwn),
      };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}
