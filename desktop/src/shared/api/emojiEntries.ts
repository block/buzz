/**
 * Catalog emoji (kind:30180, one addressable event per emoji).
 *
 * The client never loads the catalog. It resolves names with a batched `#d`
 * query over the relay socket and searches by prefix over the HTTP `/query`
 * bridge (the WebSocket path cannot carry `search_mode`).
 */

import { relayClient } from "@/shared/api/relayClient";
import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_EMOJI_ENTRY } from "@/shared/constants/kinds";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

import { normalizeShortcode } from "./customEmoji";

export { KIND_EMOJI_ENTRY };

/** Most names one `#d` filter asks for. */
export const EMOJI_ENTRY_RESOLVE_BATCH = 50;
/** Most distinct names one resolve call looks up; extra names stay plain text. */
export const EMOJI_ENTRY_RESOLVE_MAX_NAMES = 100;
/** Most results a prefix search returns. */
export const EMOJI_ENTRY_SEARCH_LIMIT = 50;

/** A shortcode that two authors published with different events. */
export type EmojiEntryConflict = {
  shortcode: string;
  /** Author of the newest event, which wins. */
  winnerPubkey: string;
  /** Other authors that published the same shortcode. */
  otherPubkeys: string[];
};

export type EmojiEntryParse = {
  emoji: CustomEmoji[];
  conflicts: EmojiEntryConflict[];
};

function isHttpUrl(url: string): boolean {
  return url.startsWith("http://") || url.startsWith("https://");
}

/** Read the one valid `{shortcode, url}` of a kind:30180 event, or null. */
function entryFromEvent(event: RelayEvent): CustomEmoji | null {
  if (event.kind !== KIND_EMOJI_ENTRY) return null;
  const d = event.tags.find((tag) => tag[0] === "d")?.[1];
  const emojiTag = event.tags.find((tag) => tag[0] === "emoji");
  if (!d || !emojiTag) return null;
  const shortcode = normalizeShortcode(d);
  if (!shortcode || shortcode !== d) return null;
  if (normalizeShortcode(emojiTag[1] ?? "") !== shortcode) return null;
  const url = emojiTag[2];
  if (!url || !isHttpUrl(url)) return null;
  return { shortcode, url };
}

/**
 * Collapse kind:30180 events to one entry per shortcode. The newest
 * `created_at` wins, per author and across authors; equal timestamps break to
 * the smaller event id so the result does not depend on fetch order. A
 * shortcode published by more than one author is reported in `conflicts`.
 */
export function parseEmojiEntryEvents(
  events: ReadonlyArray<RelayEvent>,
): EmojiEntryParse {
  const byShortcode = new Map<
    string,
    { emoji: CustomEmoji; event: RelayEvent; authors: Set<string> }
  >();
  for (const event of events) {
    const emoji = entryFromEvent(event);
    if (!emoji) continue;
    const current = byShortcode.get(emoji.shortcode);
    if (!current) {
      byShortcode.set(emoji.shortcode, {
        emoji,
        event,
        authors: new Set([event.pubkey]),
      });
      continue;
    }
    current.authors.add(event.pubkey);
    if (
      event.created_at > current.event.created_at ||
      (event.created_at === current.event.created_at &&
        event.id < current.event.id)
    ) {
      current.emoji = emoji;
      current.event = event;
    }
  }
  const conflicts: EmojiEntryConflict[] = [];
  for (const [shortcode, { event, authors }] of byShortcode) {
    if (authors.size > 1) {
      conflicts.push({
        shortcode,
        winnerPubkey: event.pubkey,
        otherPubkeys: [...authors].filter((pubkey) => pubkey !== event.pubkey),
      });
    }
  }
  if (conflicts.length > 0) {
    console.warn(
      `Emoji catalog: ${conflicts.length} shortcode(s) published by several authors; using the newest.`,
      conflicts.map((conflict) => conflict.shortcode),
    );
  }
  return {
    emoji: [...byShortcode.values()].map(({ emoji }) => emoji),
    conflicts,
  };
}

/**
 * Resolve exact names with `#d` filters of at most `EMOJI_ENTRY_RESOLVE_BATCH`
 * names. A failed query rejects; it never yields an empty result.
 */
export async function fetchEmojiEntriesByName(
  names: ReadonlyArray<string>,
  fetchEvents: (filter: {
    kinds: number[];
    "#d": string[];
    limit: number;
  }) => Promise<RelayEvent[]> = (filter) => relayClient.fetchEvents(filter),
): Promise<EmojiEntryParse> {
  const unique = [...new Set(names)].slice(0, EMOJI_ENTRY_RESOLVE_MAX_NAMES);
  const batches: string[][] = [];
  for (let i = 0; i < unique.length; i += EMOJI_ENTRY_RESOLVE_BATCH) {
    batches.push(unique.slice(i, i + EMOJI_ENTRY_RESOLVE_BATCH));
  }
  const pages = await Promise.all(
    batches.map((batch) =>
      fetchEvents({
        kinds: [KIND_EMOJI_ENTRY],
        "#d": batch,
        // Several authors can publish the same name.
        limit: batch.length * 4,
      }),
    ),
  );
  return parseEmojiEntryEvents(pages.flat());
}

/** Prefix-search the catalog over the HTTP bridge. A failed call rejects. */
export async function searchEmojiEntries(
  prefix: string,
): Promise<CustomEmoji[]> {
  const events = await invokeTauri<RelayEvent[]>("search_emoji_entries", {
    query: prefix,
    limit: EMOJI_ENTRY_SEARCH_LIMIT,
  });
  return parseEmojiEntryEvents(events).emoji;
}
