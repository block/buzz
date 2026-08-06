import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Kind-generic reads of the community catalog.
 *
 * Personas (kind:30175) and teams (kind:30178) are two projections of one
 * publishing model: a NIP-33 replaceable head per coordinate, discoverable
 * only while it carries an exact `["shared","true"]` tag. The relay gates both
 * through one `SHARED_GATED_KINDS` set, so the client-side read half is shared
 * here rather than written twice — the only thing that differs between them is
 * how the content body parses.
 */

/**
 * Whether the current identity may share this item to the catalog, and at
 * what level. `"none"` means shared with no memories attached; the persona
 * dialog and the team dialog both render it through `SnapshotOptionMenu`.
 */
export type CatalogShareLevel = "not-shared" | "none";

/**
 * A tag's value, but only when the event carries exactly one of that tag.
 *
 * Ambiguity is treated as absence: the relay's ingest rule admits exactly one
 * bounded `d` tag, so a multi-`d` event is malformed, and picking the first
 * would resolve a different coordinate than the publisher addressed.
 */
function singleTagValue(event: RelayEvent, name: string): string | null {
  const matches = event.tags.filter(
    (tag) => tag.length >= 2 && tag[0] === name && typeof tag[1] === "string",
  );
  return matches.length === 1 ? (matches[0]?.[1] ?? null) : null;
}

/**
 * Whether a catalog head is discoverable — exactly one `["shared","true"]`.
 *
 * Mirrors the relay's `event_is_shared` gate byte for byte, including the
 * two-element length check: a `["shared","true","extra"]` tag is not the tag
 * the gate admits, so treating it as shared here would show the community an
 * entry the relay will not serve.
 */
function eventIsShared(event: RelayEvent): boolean {
  const sharedTags = event.tags.filter((tag) => tag[0] === "shared");
  return (
    sharedTags.length === 1 &&
    sharedTags[0]?.length === 2 &&
    sharedTags[0]?.[1] === "true"
  );
}

function isSafeHttpUrl(value: unknown): value is string {
  // Length cap in UTF-8 bytes — same unit as the Rust is_safe_catalog_avatar_url
  // cap. This deliberately differs from `value.length` (UTF-16 code units) so
  // both sides agree for non-ASCII input (e.g. URLs with encoded emoji).
  //
  // byteLength is defined locally here (and identically in teamCatalogRelay.ts)
  // so catalogRelay.ts stays self-contained.
  function byteLength(s: string): number {
    return new TextEncoder().encode(s).length;
  }
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    byteLength(value) > 2_048 ||
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

function isInlineSvgAvatar(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.startsWith(INLINE_SVG_AVATAR_PREFIX) &&
    value.length <= MAX_INLINE_SVG_AVATAR_LENGTH
  );
}

/**
 * Shared persona heads can carry an uploaded avatar as an inline raster. Keep
 * those self-contained images renderable without accepting arbitrary `data:`
 * URLs: only the raster MIME types browsers decode in `<img>`, strict base64
 * shape, and a bound no larger than the relay's event-content ceiling.
 */
const MAX_INLINE_RASTER_AVATAR_LENGTH = 256 * 1_024;
const INLINE_RASTER_AVATAR_RE =
  /^data:image\/(?:png|jpeg|gif|webp);base64,([A-Za-z0-9+/]+={0,2})$/u;

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

/**
 * An avatar URL that is safe to hand to `<img src>`, or `null`.
 *
 * The publisher controls this string end to end, so the allowlist is the only
 * thing standing between a hostile catalog entry and a `javascript:` or
 * arbitrary-`data:` URL in the DOM. Shared by both catalog readers so a team's
 * embedded members are held to exactly the persona rule — a team projection
 * embeds N member avatars, which is N times the surface, not less.
 */
export function safeCatalogAvatarUrl(value: unknown): string | null {
  return isSafeHttpUrl(value) ||
    isInlineSvgAvatar(value) ||
    isInlineRasterAvatar(value)
    ? value
    : null;
}

/** One shared head, with the coordinate it was claimed under. */
export type CatalogHead = {
  event: RelayEvent;
  ownerPubkey: string;
  dTag: string;
};

/**
 * Collapse relay results to the canonical NIP-33 head per coordinate, then
 * keep only the shared ones.
 *
 * The relay normally returns one replaceable head. The client-side collapse is
 * defense in depth for older relays and fixtures, and deliberately claims the
 * coordinate before testing `shared` so an unshared newest head cannot
 * resurrect an older shared definition. A caller that later fails to parse a
 * claimed head must likewise drop the coordinate rather than fall back.
 */
export function sharedCatalogHeads(
  events: readonly RelayEvent[],
  kind: number,
): CatalogHead[] {
  const sorted = [...events].sort(
    (left, right) =>
      right.created_at - left.created_at || left.id.localeCompare(right.id),
  );
  const seenCoordinates = new Set<string>();
  const heads: CatalogHead[] = [];

  for (const event of sorted) {
    if (event.kind !== kind) continue;
    const dTag = singleTagValue(event, "d");
    if (!dTag) continue;
    const ownerPubkey = event.pubkey.toLowerCase();
    const coordinate = `${ownerPubkey}:${dTag}`;
    if (seenCoordinates.has(coordinate)) continue;
    seenCoordinates.add(coordinate);

    if (!eventIsShared(event)) continue;
    heads.push({ event, ownerPubkey, dTag });
  }

  return heads;
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
 * Read every event of one catalog kind, page by page.
 *
 * A single `limit`-capped fetch silently truncates once a community publishes
 * more than the relay's clamp, and the entries that fall off are simply
 * undiscoverable. Paging walks backwards through `created_at` using the only
 * cursor a WS `REQ` filter carries — `until` — which the relay treats as
 * *inclusive*, so consecutive pages overlap on tied timestamps. Two things
 * follow, and both are load-bearing:
 *
 * - dedupe by event id, because the boundary events repeat; and
 * - stop when a page contributes nothing new, because a page whose events all
 *   share one `created_at` would otherwise be requested forever.
 */
export async function fetchCatalogEvents(kind: number): Promise<RelayEvent[]> {
  const byId = new Map<string, RelayEvent>();
  let until: number | undefined;

  for (let page = 0; page < MAX_CATALOG_PAGES; page += 1) {
    const events = await relayClient.fetchEvents({
      kinds: [kind],
      limit: CATALOG_PAGE_SIZE,
      ...(until === undefined ? {} : { until }),
    });

    const sizeBefore = byId.size;
    let oldestCreatedAt = Number.POSITIVE_INFINITY;
    for (const event of events) {
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

  return [...byId.values()];
}
