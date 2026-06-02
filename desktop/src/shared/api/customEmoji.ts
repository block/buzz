/**
 * Relay-global custom emoji set (NIP-30, relay-owned).
 *
 * The authoritative emoji set is a single kind:30030 parameterized-replaceable
 * event signed by the *relay* keypair (channel_id = NULL, one canonical set —
 * the "workspace" emoji list, Slack-style). Members add/remove emoji by sending
 * a member-signed kind:9037 command; the relay validates membership and
 * re-signs the set. Clients only ever read the set and emit commands — they
 * never author the kind:30030 directly (relay ingest rejects member-authored
 * 30030).
 *
 * Contract locked with Pinky (Rust side) 2026-06-01 — see
 * PLANS/CUSTOM_EMOJI_DESKTOP.md "LOCKED CONTRACT".
 */

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/** NIP-30 emoji set (parameterized-replaceable), relay-owned. */
export const KIND_EMOJI_SET = 30030;

/** d-tag of the single canonical relay-owned set. */
export const RELAY_EMOJI_SET_D_TAG = "sprout:relay-emoji";

/**
 * Member-signed command to mutate the relay-owned set. Relay-processed (not
 * stored): the relay validates membership, applies the op, and re-signs the
 * kind:30030. Tags: `["action","set"]` + `["emoji", shortcode, url]` to
 * add/update; `["action","remove"]` + `["emoji", shortcode]` to remove.
 */
export const KIND_RELAY_EMOJI_COMMAND = 9037;

/**
 * Resolve the image URL for a reaction whose content is a custom-emoji
 * `:shortcode:`, from the relay-owned set. Returns undefined for unicode
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
 * Parse NIP-30 `["emoji", shortcode, url]` tags into a custom-emoji list.
 * Shortcodes are normalized (lowercase, no colons). Malformed/duplicate
 * entries are skipped (first wins on a collision).
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

async function fetchEmojiSetEvent(): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_EMOJI_SET],
    "#d": [RELAY_EMOJI_SET_D_TAG],
    limit: 1,
  });
  return events[events.length - 1] ?? null;
}

/** Fetch the relay-owned custom emoji set. Empty list when none published. */
export async function listCustomEmoji(): Promise<CustomEmoji[]> {
  const event = await fetchEmojiSetEvent();
  return customEmojiFromEvent(event);
}

/**
 * Add/update a custom emoji in the relay-owned set. Emits a kind:9037 command;
 * the relay validates membership and re-signs the canonical set. `url` should
 * be a Blossom blob URL (uploaded via the existing upload path). Returns the
 * normalized (lowercase) shortcode the relay will store.
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
  const event = await signRelayEvent({
    kind: KIND_RELAY_EMOJI_COMMAND,
    content: "",
    tags: [
      ["action", "set"],
      ["emoji", normalized, url],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while adding emoji.",
    "Failed to add emoji.",
  );
  return normalized;
}

/** Remove a custom emoji from the relay-owned set by shortcode. */
export async function removeCustomEmoji(shortcode: string): Promise<void> {
  const normalized = normalizeShortcode(shortcode);
  if (!normalized) return;
  const event = await signRelayEvent({
    kind: KIND_RELAY_EMOJI_COMMAND,
    content: "",
    tags: [
      ["action", "remove"],
      ["emoji", normalized],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while removing emoji.",
    "Failed to remove emoji.",
  );
}
