import { relayClient } from "@/shared/api/relayClient";
import type {
  AgentPersona,
  CatalogSourceCoordinate,
  RelayEvent,
  RespondToMode,
} from "@/shared/api/types";
import { KIND_PERSONA } from "@/shared/constants/kinds";
import { verifyEvent } from "nostr-tools/pure";

export type CatalogPersonaShareLevel = "not-shared" | "none";

type CatalogAgentProjection = {
  displayName: string;
  avatarUrl: string | null;
  systemPrompt: string;
  runtime: string | null;
  model: string | null;
  provider: string | null;
  namePool: string[];
  respondTo: RespondToMode | null;
  parallelism: number | null;
};

export type PersonaCatalogPublication = {
  eventId: string;
  ownerPubkey: string;
  sourcePersonaId: string;
  createdAt: number;
  agent: CatalogAgentProjection;
};

export type CatalogPersona = AgentPersona & {
  catalogSource: CatalogSourceCoordinate & {
    /** The publication event this projection was built from. */
    eventId: string;
    /** Whether the current identity published it. */
    isOwn: boolean;
  };
};

type JsonObject = Record<string, unknown>;

const MAX_AGENT_DISPLAY_NAME_CHARACTERS = 128;
const MAX_AGENT_SYSTEM_PROMPT_BYTES = 64 * 1_024;
const EMOJI_VARIATION_SELECTOR = 0xfe0f;
const ZERO_WIDTH_JOINER = 0x200d;
const EXTENDED_PICTOGRAPHIC_RE = /^\p{Extended_Pictographic}$/u;

function isProhibitedAgentTextCharacter(
  characters: readonly string[],
  index: number,
  allowLayoutControls: boolean,
): boolean {
  const character = characters[index];
  if (character === undefined) return false;
  const codePoint = character.codePointAt(0);
  if (codePoint === undefined) return false;

  const isControl =
    codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
  const isAllowedLayoutControl =
    allowLayoutControls && (codePoint === 0x09 || codePoint === 0x0a);
  if (isControl && !isAllowedLayoutControl) return true;
  if (isAllowedEmojiFormatCharacter(characters, index)) return false;

  return (
    codePoint === 0x00ad ||
    codePoint === 0x034f ||
    codePoint === 0x061c ||
    (codePoint >= 0x115f && codePoint <= 0x1160) ||
    (codePoint >= 0x17b4 && codePoint <= 0x17b5) ||
    (codePoint >= 0x180b && codePoint <= 0x180f) ||
    (codePoint >= 0x200b && codePoint <= 0x200f) ||
    (codePoint >= 0x202a && codePoint <= 0x202e) ||
    (codePoint >= 0x2060 && codePoint <= 0x206f) ||
    codePoint === 0x3164 ||
    (codePoint >= 0xfe00 && codePoint <= 0xfe0f) ||
    codePoint === 0xfeff ||
    codePoint === 0xffa0 ||
    (codePoint >= 0xfff0 && codePoint <= 0xfff8) ||
    (codePoint >= 0x1bca0 && codePoint <= 0x1bca3) ||
    (codePoint >= 0x1d173 && codePoint <= 0x1d17a) ||
    (codePoint >= 0xe0000 && codePoint <= 0xe0fff)
  );
}

function isAllowedEmojiFormatCharacter(
  characters: readonly string[],
  index: number,
): boolean {
  const codePoint = characters[index]?.codePointAt(0);
  if (codePoint === EMOJI_VARIATION_SELECTOR) {
    const previous = characters[index - 1];
    return previous !== undefined && isEmojiVariationBase(previous);
  }
  if (codePoint !== ZERO_WIDTH_JOINER) return false;

  const next = characters[index + 1];
  return (
    hasPrecedingEmojiBase(characters, index) &&
    next !== undefined &&
    EXTENDED_PICTOGRAPHIC_RE.test(next)
  );
}

function hasPrecedingEmojiBase(
  characters: readonly string[],
  index: number,
): boolean {
  for (let previous = index - 1; previous >= 0; previous -= 1) {
    const character = characters[previous];
    const codePoint = character?.codePointAt(0);
    if (
      codePoint === EMOJI_VARIATION_SELECTOR ||
      (codePoint !== undefined && codePoint >= 0x1f3fb && codePoint <= 0x1f3ff)
    ) {
      continue;
    }
    return character !== undefined && EXTENDED_PICTOGRAPHIC_RE.test(character);
  }
  return false;
}

function isEmojiVariationBase(character: string): boolean {
  return (
    /^[#*0-9]$/u.test(character) || EXTENDED_PICTOGRAPHIC_RE.test(character)
  );
}

function isSafeAgentDefinitionText(
  displayName: string,
  systemPrompt: string,
): boolean {
  const displayNameCharacters = [...displayName];
  const systemPromptCharacters = [...systemPrompt];
  return (
    displayName.trim().length > 0 &&
    displayNameCharacters.length <= MAX_AGENT_DISPLAY_NAME_CHARACTERS &&
    new TextEncoder().encode(systemPrompt).length <=
      MAX_AGENT_SYSTEM_PROMPT_BYTES &&
    !displayNameCharacters.some((_character, index) =>
      isProhibitedAgentTextCharacter(displayNameCharacters, index, false),
    ) &&
    !systemPromptCharacters.some((_character, index) =>
      isProhibitedAgentTextCharacter(systemPromptCharacters, index, true),
    )
  );
}

function eventHasValidSignature(event: RelayEvent): boolean {
  try {
    // Verify a fresh wire-shaped value. nostr-tools memoizes successful checks
    // on event objects; relay input must never inherit a stale verification
    // marker from an object that was subsequently mutated.
    return verifyEvent({
      id: event.id,
      pubkey: event.pubkey,
      created_at: event.created_at,
      kind: event.kind,
      tags: event.tags,
      content: event.content,
      sig: event.sig,
    });
  } catch {
    return false;
  }
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function extractTag(event: RelayEvent, name: string): string | null {
  const matches = event.tags.filter(
    (tag) => tag.length >= 2 && tag[0] === name && typeof tag[1] === "string",
  );
  return matches.length === 1 ? (matches[0]?.[1] ?? null) : null;
}

export function personaEventIsShared(event: RelayEvent): boolean {
  const sharedTags = event.tags.filter((tag) => tag[0] === "shared");
  return (
    sharedTags.length === 1 &&
    sharedTags[0]?.length === 2 &&
    sharedTags[0]?.[1] === "true"
  );
}

function isSafeHttpUrl(value: unknown): value is string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 2_048 ||
    /[\s()]/u.test(value)
  ) {
    return false;
  }
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:" || parsed.protocol === "http:";
  } catch {
    return false;
  }
}

/**
 * Emoji avatars are the one `data:` avatar a catalog entry keeps.
 *
 * They persist as inline, percent-encoded SVG (`emojiAvatarDataUrl` in
 * `ProfileAvatarEditor.utils.ts`), so they are self-contained and render on
 * any member's machine — unlike a bundled runtime-default avatar, whose local
 * asset path means nothing to another install. The accepted shape is exactly
 * that prefix: the trailing comma is what rejects `;base64` payloads, and
 * every other `data:` MIME stays rejected. Catalog avatars render through
 * `<img src>` (`ProfileAvatar` → `AvatarImage`), where SVG script never
 * executes, so bounding the length is the remaining concern — 8 KiB is an
 * order of magnitude above the ~700 characters an emoji avatar encodes to.
 */
const INLINE_SVG_AVATAR_PREFIX = "data:image/svg+xml,";
const MAX_INLINE_SVG_AVATAR_LENGTH = 8_192;

/**
 * Shared persona heads can carry an uploaded avatar as an inline raster. Keep
 * those self-contained images renderable without accepting arbitrary `data:`
 * URLs: only the raster MIME types browsers decode in `<img>`, strict base64
 * shape, and a bound no larger than the relay's event-content ceiling.
 */
const MAX_INLINE_RASTER_AVATAR_LENGTH = 256 * 1_024;
const INLINE_RASTER_AVATAR_RE =
  /^data:image\/(?:png|jpeg|gif|webp);base64,([A-Za-z0-9+/]+={0,2})$/u;

function isInlineSvgAvatar(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.startsWith(INLINE_SVG_AVATAR_PREFIX) &&
    value.length <= MAX_INLINE_SVG_AVATAR_LENGTH
  );
}

function isInlineRasterAvatar(value: unknown): value is string {
  if (
    typeof value !== "string" ||
    value.length > MAX_INLINE_RASTER_AVATAR_LENGTH
  ) {
    return false;
  }
  const match = INLINE_RASTER_AVATAR_RE.exec(value);
  return match !== null && (match[1]?.length ?? 0) % 4 === 0;
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function parsePersonaContent(event: RelayEvent): CatalogAgentProjection | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (!isObject(parsed)) return null;

  const displayName = parsed.display_name;
  const systemPrompt =
    typeof parsed.system_prompt === "string" ? parsed.system_prompt : "";
  if (
    typeof displayName !== "string" ||
    !isSafeAgentDefinitionText(displayName, systemPrompt)
  ) {
    return null;
  }

  const avatarUrl =
    isSafeHttpUrl(parsed.avatar_url) ||
    isInlineSvgAvatar(parsed.avatar_url) ||
    isInlineRasterAvatar(parsed.avatar_url)
      ? parsed.avatar_url
      : null;
  const namePool = Array.isArray(parsed.name_pool)
    ? parsed.name_pool.filter(
        (candidate): candidate is string => typeof candidate === "string",
      )
    : [];
  const respondTo =
    parsed.respond_to === "allowlist"
      ? "owner-only"
      : parsed.respond_to === "owner-only" || parsed.respond_to === "anyone"
        ? parsed.respond_to
        : null;
  const parallelism =
    typeof parsed.parallelism === "number" &&
    Number.isInteger(parsed.parallelism) &&
    parsed.parallelism >= 1 &&
    parsed.parallelism <= 32
      ? parsed.parallelism
      : null;

  return {
    displayName,
    avatarUrl,
    systemPrompt,
    runtime: optionalString(parsed.runtime),
    model: optionalString(parsed.model),
    provider: optionalString(parsed.provider),
    namePool,
    respondTo,
    parallelism,
  };
}

/**
 * Collapse relay results to the canonical NIP-33 head for each persona
 * coordinate, then keep only exact `["shared", "true"]` heads.
 *
 * The relay normally returns one replaceable head. The client-side collapse is
 * defense in depth for older relays and fixtures, and deliberately claims the
 * coordinate before parsing so an invalid or unshared newest head cannot
 * resurrect an older shared definition.
 */
export function catalogPublicationsFromEvents(
  events: readonly RelayEvent[],
): PersonaCatalogPublication[] {
  return catalogPublicationsFromVerifiedEvents(
    events.filter(eventHasValidSignature),
  );
}

function catalogPublicationsFromVerifiedEvents(
  events: readonly RelayEvent[],
): PersonaCatalogPublication[] {
  const sorted = [...events].sort(
    (left, right) =>
      right.created_at - left.created_at || left.id.localeCompare(right.id),
  );
  const seenCoordinates = new Set<string>();
  const publications: PersonaCatalogPublication[] = [];

  for (const event of sorted) {
    if (event.kind !== KIND_PERSONA) continue;
    const sourcePersonaId = extractTag(event, "d");
    if (!sourcePersonaId) continue;
    const ownerPubkey = event.pubkey.toLowerCase();
    const coordinate = `${ownerPubkey}:${sourcePersonaId}`;
    if (seenCoordinates.has(coordinate)) continue;
    seenCoordinates.add(coordinate);

    if (!personaEventIsShared(event)) continue;
    const agent = parsePersonaContent(event);
    if (!agent) continue;
    publications.push({
      eventId: event.id,
      ownerPubkey,
      sourcePersonaId,
      createdAt: event.created_at,
      agent,
    });
  }

  return publications;
}

/**
 * Events per catalog page.
 *
 * Kept well under the relay's 1,000-row `query_events` clamp so a page that
 * comes back full is a reliable "there may be more" signal rather than a
 * silently truncated result.
 */
const CATALOG_PAGE_SIZE = 500;

/**
 * Hard bound on pages walked, so a relay that keeps returning full pages can
 * never spin this forever.
 */
const MAX_CATALOG_PAGES = 40;

/**
 * Read every shared persona event, page by page.
 *
 * A single `limit`-capped fetch silently truncates once a community publishes
 * more agents than the relay's clamp, and the entries that fall off are simply
 * undiscoverable. Paging walks backwards through `created_at` using the only
 * cursor a WS `REQ` filter carries — `until` — which the relay treats as
 * *inclusive*, so consecutive pages overlap on tied timestamps. Two things
 * follow, and both are load-bearing:
 *
 * - dedupe by event id, because the boundary events repeat; and
 * - stop when a page contributes nothing new, because a page whose events all
 *   share one `created_at` would otherwise be requested forever.
 */
export async function fetchPersonaCatalogPublications(): Promise<
  PersonaCatalogPublication[]
> {
  const byId = new Map<string, RelayEvent>();
  let until: number | undefined;

  for (let page = 0; page < MAX_CATALOG_PAGES; page += 1) {
    const events = await relayClient.fetchEvents({
      kinds: [KIND_PERSONA],
      limit: CATALOG_PAGE_SIZE,
      ...(until === undefined ? {} : { until }),
    });

    const sizeBefore = byId.size;
    let oldestCreatedAt = Number.POSITIVE_INFINITY;
    for (const event of events) {
      if (!eventHasValidSignature(event)) continue;
      byId.set(event.id, event);
      oldestCreatedAt = Math.min(oldestCreatedAt, event.created_at);
    }

    // A short page is the end of the catalog; a page of only-repeats means the
    // cursor cannot advance past a run of tied timestamps.
    if (events.length < CATALOG_PAGE_SIZE || byId.size === sizeBefore) {
      break;
    }
    until = oldestCreatedAt;
  }

  return catalogPublicationsFromVerifiedEvents([...byId.values()]);
}

function publicationToPersona(
  publication: PersonaCatalogPublication,
  localPersona: AgentPersona | undefined,
  isOwn: boolean,
): CatalogPersona {
  const timestamp = new Date(publication.createdAt * 1_000).toISOString();
  // The publication remains authoritative for catalog presentation. An added
  // local copy contributes only the linkage id and selected state; merging the
  // whole copy would leak local edits (notably its avatar) into the publisher's
  // catalog entry.
  const basePersona: AgentPersona = {
    id:
      localPersona?.id ??
      `catalog:${publication.ownerPubkey}:${publication.sourcePersonaId}`,
    displayName: publication.agent.displayName,
    avatarUrl: publication.agent.avatarUrl,
    systemPrompt: publication.agent.systemPrompt,
    runtime: publication.agent.runtime,
    model: publication.agent.model,
    provider: publication.agent.provider,
    namePool: publication.agent.namePool,
    isBuiltIn: false,
    isActive: localPersona?.isActive ?? false,
    shared: true,
    sourceTeam: null,
    envVars: {},
    respondTo: publication.agent.respondTo,
    respondToAllowlist: [],
    parallelism: publication.agent.parallelism,
    createdAt: timestamp,
    updatedAt: timestamp,
  };

  return {
    ...basePersona,
    // Catalog membership is relay-confirmed by the shared event itself. Do not
    // let a local pending toggle override this projection.
    shared: true,
    catalogSource: {
      eventId: publication.eventId,
      ownerPubkey: publication.ownerPubkey,
      isOwn,
      personaId: publication.sourcePersonaId,
    },
  };
}

export function catalogPersonasFromPublications(
  publications: readonly PersonaCatalogPublication[],
  localPersonas: readonly AgentPersona[],
  currentPubkey: string | null | undefined,
): CatalogPersona[] {
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase() ?? null;
  const personas: CatalogPersona[] = [];

  for (const publication of publications) {
    const isOwn = publication.ownerPubkey === normalizedCurrentPubkey;
    personas.push(
      publicationToPersona(
        publication,
        findLocalPersonaForCatalogEntry(localPersonas, {
          ownerPubkey: publication.ownerPubkey,
          personaId: publication.sourcePersonaId,
          isOwn,
        }),
        isOwn,
      ),
    );
  }

  return personas.sort((left, right) =>
    left.displayName.localeCompare(right.displayName),
  );
}

/**
 * The local persona backing a catalog entry, if the user already has it.
 *
 * An own publication is found by id — its `d`-tag *is* the local persona id. A
 * copy of another owner's entry carries a fresh local id instead, so the only
 * link back is the `catalogSource` coordinate stored on the copy. Matching on
 * that coordinate is what stops the catalog from offering "Add" for an entry
 * the user already added, which would mint a second copy.
 */
export function findLocalPersonaForCatalogEntry(
  localPersonas: readonly AgentPersona[],
  source: CatalogSourceCoordinate & { isOwn: boolean },
): AgentPersona | undefined {
  if (source.isOwn) {
    return localPersonas.find((persona) => persona.id === source.personaId);
  }
  return localPersonas.find(
    (persona) =>
      persona.catalogSource?.ownerPubkey === source.ownerPubkey &&
      persona.catalogSource?.personaId === source.personaId,
  );
}

export function isCatalogPersona(
  persona: AgentPersona,
): persona is CatalogPersona {
  return "catalogSource" in persona && isObject(persona.catalogSource);
}
