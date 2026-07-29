import type { RelayEvent } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";

const HEX_RE = /^[0-9a-f]+$/i;

function reactionTargetId(tags: string[][]): string | null {
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const tag = tags[index];
    if (
      tag?.[0] === "e" &&
      typeof tag[1] === "string" &&
      tag[1].length === 64 &&
      HEX_RE.test(tag[1])
    ) {
      return tag[1];
    }
  }
  return null;
}

export type MessageReaction = {
  emoji: string;
  /** Who reacted — names the reactor in the chip's hover tooltip. */
  pubkey: string;
};

/**
 * Aggregate kind-7 reaction events by target message. Agents react to a
 * prompt while working on it, so these chips double as the loading state.
 */
export function collectReactions(
  events: RelayEvent[] | undefined,
): Map<string, MessageReaction[]> {
  const byTarget = new Map<string, MessageReaction[]>();
  for (const event of events ?? []) {
    if (event.kind !== KIND_REACTION) continue;
    const targetId = reactionTargetId(event.tags);
    if (!targetId) continue;
    const raw = event.content.trim();
    const emoji = raw === "" || raw === "+" ? "👍" : raw;
    const reaction = { emoji, pubkey: event.pubkey };
    const bucket = byTarget.get(targetId);
    if (bucket) {
      bucket.push(reaction);
    } else {
      byTarget.set(targetId, [reaction]);
    }
  }
  return byTarget;
}
