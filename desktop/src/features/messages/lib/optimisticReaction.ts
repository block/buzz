import type { RelayEvent } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";

/**
 * Synthesize the kind:7 reaction event a freshly-clicked reaction produces, so
 * it can be written into the channel cache immediately rather than waiting for
 * a relay round-trip that never comes: reactions carry only an `e` tag (no
 * `h`), so the `#h`-scoped live subscription (`buildChannelFilter`) never
 * delivers them back — they only arrive via the cold-load `#e` aux backfill.
 * Without this the reactor's own reaction stays invisible until the next
 * channel switch, and re-clicking just re-hits the relay's duplicate guard.
 *
 * The shape mirrors what `formatTimelineMessages` reads: the target `e` tag,
 * the emoji as content, the optional NIP-30 `["emoji", shortcode, url]` tag for
 * custom emoji, and the actor's pubkey. Reaction render dedupes by
 * `targetId:actorPubkey:emoji`, so when the real event later loads via backfill
 * it collapses onto this one — no double count.
 */
export function createOptimisticReaction(
  targetEventId: string,
  emoji: string,
  emojiUrl: string | undefined,
  pubkey: string,
): RelayEvent {
  const tags: string[][] = [["e", targetEventId]];
  // Custom-emoji reaction (NIP-30): content is `:shortcode:` with the image URL
  // on a matching `emoji` tag.
  if (emojiUrl && emoji.startsWith(":") && emoji.endsWith(":")) {
    tags.push(["emoji", emoji.slice(1, -1), emojiUrl]);
  }

  return {
    id: `optimistic-reaction-${crypto.randomUUID()}`,
    pubkey,
    created_at: Math.floor(Date.now() / 1_000),
    kind: KIND_REACTION,
    tags,
    content: emoji,
    sig: "",
  };
}
