/**
 * Relay-global custom emoji set (NIP-30, relay-owned).
 *
 * The authoritative emoji set is a single kind:30030 parameterized-replaceable
 * event signed by the *relay* keypair (channel_id = NULL, one canonical set —
 * the "workspace" emoji list, Slack-style). Members add/remove emoji by sending
 * a relay-processed command event; the relay validates membership and re-signs
 * the set. Clients only ever read the set and emit the command — they never
 * author the kind:30030 directly.
 *
 * Mirrors `relayMembers.ts` (NIP-43 relay-signed global list) for fetch/parse.
 *
 * NOTE (integration point with Pinky / crates side): the d-tag constant
 * (`RELAY_EMOJI_SET_D_TAG`) and the add/remove command kind + tag shape are
 * owned by the Rust contract. Values below match the locked plan; confirm
 * against `crates/**` before the client emits commands.
 */

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/** NIP-30 emoji set (parameterized-replaceable). */
export const KIND_EMOJI_SET = 30030;

/**
 * Fixed d-tag for the single relay-owned set. The relay is the only author of
 * kind:30030, so (kind, relay_pubkey, d_tag) addresses exactly one event.
 */
export const RELAY_EMOJI_SET_D_TAG = "sprout:relay-emoji";

/**
 * Member command to mutate the relay-owned set. Relay-processed (not stored as
 * a regular event): the relay validates membership, applies the op, re-signs
 * the kind:30030. TODO(pinky): confirm kind number + tag shape against crates.
 */
export const KIND_EMOJI_COMMAND = 9040;

const SHORTCODE_RE = /^[a-z0-9_+-]+$/i;

/**
 * Parse a relay-owned kind:30030 event into the custom-emoji list. NIP-30 body
 * tags are `["emoji", shortcode, url]`. Malformed/duplicate entries are skipped
 * (first writer wins on a shortcode collision within the single set).
 */
export function customEmojiFromEvent(event: RelayEvent | null): CustomEmoji[] {
  if (!event) return [];
  const seen = new Set<string>();
  const emoji: CustomEmoji[] = [];

  for (const tag of event.tags) {
    const [name, shortcode, url] = tag;
    if (name !== "emoji") continue;
    if (!shortcode || !url) continue;
    if (!SHORTCODE_RE.test(shortcode)) continue;
    if (seen.has(shortcode)) continue;
    seen.add(shortcode);
    emoji.push({ shortcode, url });
  }

  return emoji;
}

async function fetchEmojiSetEvent(): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_EMOJI_SET],
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
 * Add a custom emoji to the relay-owned set. Emits the member command; the
 * relay validates membership and re-signs the canonical set.
 * `url` should be a Blossom blob URL (uploaded via the existing upload path).
 */
export async function addCustomEmoji(
  shortcode: string,
  url: string,
): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_EMOJI_COMMAND,
    content: "",
    tags: [["emoji", shortcode, url, "add"]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while adding emoji.",
    "Failed to add emoji.",
  );
}

/** Remove a custom emoji from the relay-owned set by shortcode. */
export async function removeCustomEmoji(shortcode: string): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_EMOJI_COMMAND,
    content: "",
    tags: [["emoji", shortcode, "", "remove"]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while removing emoji.",
    "Failed to remove emoji.",
  );
}
