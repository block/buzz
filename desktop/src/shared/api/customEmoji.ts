/**
 * Workspace custom emoji (NIP-30, per-user sets).
 *
 * Each member publishes their OWN kind:30030 parameterized-replaceable event,
 * signed as themselves, keyed by `(pubkey, 30030, "sprout:custom-emoji")`. The
 * "workspace palette" shown in the picker/renderer is the client-side UNION of
 * every member's set, deduped by `(shortcode, url)` — a view computed on read,
 * not stored state. Adding an emoji is a read-my-own-set → mutate → republish
 * of my own 30030 (relay ingest allowlists member-authored 30030/10030 as
 * UsersWrite, and the generic NIP-33 replace path keeps only the latest per
 * `(pubkey, d_tag)`).
 *
 * Replaces the earlier relay-owned single-set + kind:9037 command model.
 */

import { relayClient } from "@/shared/api/relayClient";
import { getIdentity, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/** NIP-30 emoji set (parameterized-replaceable). */
export const KIND_EMOJI_SET = 30030;

/** d-tag for a member's own custom emoji set. */
export const CUSTOM_EMOJI_SET_D_TAG = "sprout:custom-emoji";

/**
 * Resolve the image URL for a reaction whose content is a custom-emoji
 * `:shortcode:`, from the workspace set. Returns undefined for unicode
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

/**
 * Normalize a shortcode the same way the relay does: strip surrounding colons
 * and lowercase. Returns null if the result is empty or has invalid chars.
 */
export function normalizeShortcode(raw: string): string | null {
  const stripped = raw.trim().replace(/^:+/, "").replace(/:+$/, "");
  const lower = stripped.toLowerCase();
  return SHORTCODE_RE.test(lower) ? lower : null;
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

/**
 * Union every member's kind:30030 set into the workspace palette, deduped by
 * `(shortcode, url)`. Stable order: sorted by shortcode then url, so the same
 * set of events always yields the same list (no picker reshuffle).
 */
export function unionCustomEmoji(
  events: ReadonlyArray<RelayEvent>,
): CustomEmoji[] {
  const seen = new Set<string>();
  const out: CustomEmoji[] = [];
  for (const event of events) {
    for (const { shortcode, url } of customEmojiFromTags(event.tags)) {
      const key = `${shortcode}\u0000${url}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ shortcode, url });
    }
  }
  out.sort(
    (a, b) =>
      a.shortcode.localeCompare(b.shortcode) || a.url.localeCompare(b.url),
  );
  return out;
}

/** Fetch every member's 30030 set (catch-up). */
export async function fetchWorkspaceEmojiEvents(): Promise<RelayEvent[]> {
  return relayClient.fetchEvents({
    kinds: [KIND_EMOJI_SET],
    "#d": [CUSTOM_EMOJI_SET_D_TAG],
    // One 30030 per member; a workspace has far fewer than this. The relay
    // already keeps only the latest per (pubkey, d_tag), so this is the member
    // count, not history depth.
    limit: 500,
  });
}

/** Fetch the workspace custom emoji palette (union). Empty when none. */
export async function listCustomEmoji(): Promise<CustomEmoji[]> {
  const events = await fetchWorkspaceEmojiEvents();
  return unionCustomEmoji(events);
}

/** Fetch the caller's OWN current set (latest 30030 under the d-tag). */
async function fetchOwnEmoji(): Promise<CustomEmoji[]> {
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
