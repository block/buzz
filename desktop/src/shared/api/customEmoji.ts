/**
 * Community custom emoji (NIP-30, per-user sets).
 *
 * Each member publishes their OWN kind:30030 parameterized-replaceable event,
 * signed as themselves, keyed by `(pubkey, 30030, "buzz:custom-emoji")`. The
 * "community palette" shown in the picker/renderer is the client-side UNION of
 * every member's set, collapsed to one entry per shortcode (deterministic
 * winner) — a view computed on read, not stored state. Downstream identity is
 * shortcode-only (emoji-mart id, autocomplete key, reaction lookup, send tag),
 * so the palette must never expose two URLs under one shortcode. Adding an
 * emoji is a read-my-own-set → mutate → republish
 * of my own 30030 (relay ingest allowlists member-authored 30030/10030 as
 * UsersWrite, and the generic NIP-33 replace path keeps only the latest per
 * `(pubkey, d_tag)`).
 *
 * Replaces the earlier relay-owned single-set + kind:9037 command model.
 */

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type { RelayEvent } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/** NIP-30 emoji set (parameterized-replaceable). */
export const KIND_EMOJI_SET = 30030;

/** d-tag for a member's own custom emoji set. */
export const CUSTOM_EMOJI_SET_D_TAG = "buzz:custom-emoji";

/**
 * Resolve the image URL for a reaction whose content is a custom-emoji
 * `:shortcode:`, from the community set. Returns undefined for unicode
 * reactions or unknown shortcodes (the kind:7 then carries no emoji tag).
 */
export function reactionEmojiUrl(
  emoji: string,
  set: ReadonlyArray<CustomEmoji> | undefined,
): string | undefined {
  if (!set || !emoji.startsWith(":") || !emoji.endsWith(":")) return undefined;
  const shortcode = emoji.slice(1, -1).toLowerCase();
  return set.find((e) => e.shortcode === shortcode)?.url;
}

/** NIP-30 shortcode chars. Matches the relay's `[A-Za-z0-9_-]` validation. */
const SHORTCODE_RE = /^[a-z0-9_-]+$/;
const MAX_SHORTCODE_LENGTH = 64;

/**
 * Normalize a shortcode the same way the relay does: strip surrounding colons
 * and lowercase. Returns null if the result is empty or has invalid chars.
 */
export function normalizeShortcode(raw: string): string | null {
  const stripped = raw.trim().replace(/^:+/, "").replace(/:+$/, "");
  const lower = stripped.toLowerCase();
  return lower.length <= MAX_SHORTCODE_LENGTH && SHORTCODE_RE.test(lower)
    ? lower
    : null;
}

/**
 * Suggest a valid custom-emoji shortcode from an uploaded filename.
 * Mirrors Slack's file-first flow: strip the extension, lowercase, and collapse
 * runs of invalid characters into a single underscore.
 */
export function suggestShortcodeFromFilename(filename: string): string | null {
  const basename = filename
    .trim()
    .replace(/^.*[/\\]/, "")
    .replace(/\.[^.]*$/, "");
  const suggested = basename
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^[_-]+|[_-]+$/g, "");
  return normalizeShortcode(suggested);
}

/**
 * Parse NIP-30 `["emoji", shortcode, url]` tags from a single event into a
 * custom-emoji list. Shortcodes are normalized; malformed/duplicate entries
 * within the one event are skipped (first wins).
 */
export function customEmojiFromTags(
  tags: ReadonlyArray<ReadonlyArray<string>>,
): CustomEmoji[] {
  const seen = new Set<string>();
  const emoji: CustomEmoji[] = [];

  for (const tag of tags) {
    const [name, rawShortcode, url] = tag;
    if (name !== "emoji") continue;
    if (!rawShortcode || !url) continue;
    const shortcode = normalizeShortcode(rawShortcode);
    if (!shortcode) continue;
    if (seen.has(shortcode)) continue;
    seen.add(shortcode);
    emoji.push({ shortcode, url });
  }

  return emoji;
}

export function customEmojiFromEvent(event: RelayEvent | null): CustomEmoji[] {
  if (!event) return [];
  return customEmojiFromTags(event.tags);
}

/** Most 30030 sets one catalog fetch asks the relay for. */
export const COMMUNITY_EMOJI_SET_LIMIT = 500;

/**
 * Most entries the union keeps. At about 160 UTF-16 code units per entry
 * (shortcode, Blossom URL, JSON framing) this is roughly 1.3 MB, under the
 * 2 MiB pure-cache budget in `localStorageQuota.ts`, and it keeps the composer
 * regex and the picker list small.
 */
export const COMMUNITY_EMOJI_ENTRY_CAP = 4000;

export type CommunityEmojiCatalog = {
  emoji: CustomEmoji[];
  /** Number of 30030 sets the union read. */
  setCount: number;
  /** True when the relay returned exactly the requested limit, so sets may be missing. */
  setLimitReached: boolean;
  /** Entries left out because the union passed `COMMUNITY_EMOJI_ENTRY_CAP`. */
  droppedEntries: number;
};

/**
 * Union every member's kind:30030 set into the community palette, collapsed to
 * one entry per shortcode. When members disagree on a shortcode's URL, the
 * most recently published set wins (`created_at` is signed event data, so this
 * is as deterministic and fetch-order-independent as any pure function of the
 * events); equal timestamps tie-break to the lexicographically-smallest URL so
 * the same set of events always yields the same palette. Output is sorted by
 * shortcode. Past `COMMUNITY_EMOJI_ENTRY_CAP` entries, the first entries in that
 * shortcode order are kept and the rest are counted in `droppedEntries`.
 */
export function buildCommunityEmojiCatalog(
  events: ReadonlyArray<RelayEvent>,
): CommunityEmojiCatalog {
  const byShortcode = new Map<string, { url: string; createdAt: number }>();
  for (const event of events) {
    for (const { shortcode, url } of customEmojiFromTags(event.tags)) {
      const winner = byShortcode.get(shortcode);
      if (
        winner === undefined ||
        event.created_at > winner.createdAt ||
        (event.created_at === winner.createdAt && url < winner.url)
      ) {
        byShortcode.set(shortcode, { url, createdAt: event.created_at });
      }
    }
  }
  const sorted = [...byShortcode]
    .map(([shortcode, { url }]) => ({ shortcode, url }))
    .sort((a, b) => a.shortcode.localeCompare(b.shortcode));
  const droppedEntries = Math.max(0, sorted.length - COMMUNITY_EMOJI_ENTRY_CAP);
  if (droppedEntries > 0) {
    console.warn(
      `Custom emoji catalog has ${sorted.length} entries; dropped ${droppedEntries} past the ${COMMUNITY_EMOJI_ENTRY_CAP} cap.`,
    );
  }
  const setLimitReached = events.length >= COMMUNITY_EMOJI_SET_LIMIT;
  if (setLimitReached) {
    console.warn(
      `Custom emoji fetch returned ${events.length} sets, the request limit; some sets may be missing.`,
    );
  }
  return {
    emoji:
      droppedEntries > 0 ? sorted.slice(0, COMMUNITY_EMOJI_ENTRY_CAP) : sorted,
    setCount: events.length,
    setLimitReached,
    droppedEntries,
  };
}

/** Union of member sets as a plain list; see `buildCommunityEmojiCatalog`. */
export function unionCustomEmoji(
  events: ReadonlyArray<RelayEvent>,
): CustomEmoji[] {
  return buildCommunityEmojiCatalog(events).emoji;
}

/** Fetch every member's 30030 set (catch-up). */
export async function fetchCommunityEmojiEvents(): Promise<RelayEvent[]> {
  return relayClient.fetchEvents({
    kinds: [KIND_EMOJI_SET],
    "#d": [CUSTOM_EMOJI_SET_D_TAG],
    // The relay keeps only the latest per (pubkey, d_tag), so this counts members.
    limit: COMMUNITY_EMOJI_SET_LIMIT,
  });
}

/** Fetch the community catalog. A failed fetch rejects; it never yields an empty catalog. */
export async function fetchCommunityEmojiCatalog(): Promise<CommunityEmojiCatalog> {
  return buildCommunityEmojiCatalog(await fetchCommunityEmojiEvents());
}

/** Fetch the community custom emoji palette (union). Empty when none. */
export async function listCustomEmoji(): Promise<CustomEmoji[]> {
  return (await fetchCommunityEmojiCatalog()).emoji;
}

/** Fetch the caller's OWN current set (latest 30030 under the d-tag). */
export async function fetchOwnEmoji(): Promise<CustomEmoji[]> {
  const { pubkey: me } = await getIdentity();
  if (!me) return [];
  const events = await relayClient.fetchEvents({
    kinds: [KIND_EMOJI_SET],
    "#d": [CUSTOM_EMOJI_SET_D_TAG],
    authors: [me],
    limit: 1,
  });
  return customEmojiFromEvent(events[events.length - 1] ?? null);
}

/** Publish the caller's (replaced) own 30030 set, signed as the caller. */
async function publishOwnSet(
  emojis: ReadonlyArray<CustomEmoji>,
  timeoutMessage: string,
  errorMessage: string,
): Promise<void> {
  const tags: string[][] = [["d", CUSTOM_EMOJI_SET_D_TAG]];
  for (const { shortcode, url } of emojis) {
    tags.push(["emoji", shortcode, url]);
  }
  const event = await signRelayEvent({
    kind: KIND_EMOJI_SET,
    content: "",
    tags,
  });
  await relayClient.publishEvent(event, timeoutMessage, errorMessage);
}

/**
 * Add/update a custom emoji in the caller's OWN set (read-modify-write).
 * `url` should be a Blossom blob URL. Returns the normalized shortcode.
 */
export async function setCustomEmoji(
  shortcode: string,
  url: string,
): Promise<string> {
  const normalized = normalizeShortcode(shortcode);
  if (!normalized) {
    throw new Error(
      "Invalid emoji name. Use letters, numbers, hyphen, or underscore.",
    );
  }
  const own = await fetchOwnEmoji();
  const next = own.filter((e) => e.shortcode !== normalized);
  next.push({ shortcode: normalized, url });
  await publishOwnSet(
    next,
    "Timed out while adding emoji.",
    "Failed to add emoji.",
  );
  return normalized;
}

/** Remove a custom emoji from the caller's OWN set by shortcode. */
export async function removeCustomEmoji(shortcode: string): Promise<void> {
  const normalized = normalizeShortcode(shortcode);
  if (!normalized) return;
  const own = await fetchOwnEmoji();
  const next = own.filter((e) => e.shortcode !== normalized);
  if (next.length === own.length) return; // not present — nothing to republish
  await publishOwnSet(
    next,
    "Timed out while removing emoji.",
    "Failed to remove emoji.",
  );
}
