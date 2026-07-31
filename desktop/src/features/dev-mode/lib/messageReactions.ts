import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_NIP29_DELETE_EVENT,
  KIND_REACTION,
} from "@/shared/constants/kinds";

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
  /** Custom-emoji image URL from the reaction's NIP-30 `emoji` tag. */
  emojiUrl?: string;
  /** Who reacted — names the reactor in the chip's hover tooltip. */
  pubkey: string;
};

/** One rendered chip: everyone who reacted with the same emoji. */
export type ReactionGroup = {
  emoji: string;
  emojiUrl?: string;
  pubkeys: string[];
};

/**
 * A custom-emoji reaction's content is `:shortcode:` and its image URL rides
 * on a matching NIP-30 `["emoji", shortcode, url]` tag.
 */
function reactionEmojiTagUrl(
  emoji: string,
  tags: string[][],
): string | undefined {
  if (!emoji.startsWith(":") || !emoji.endsWith(":")) return undefined;
  const shortcode = emoji.slice(1, -1);
  return tags.find(
    (tag) => tag[0] === "emoji" && tag[1] === shortcode && tag[2],
  )?.[2];
}

/**
 * Aggregate kind-7 reaction events by target message. Agents react to a
 * prompt while working on it, so these chips double as the loading state.
 * Reactions withdrawn via a kind-5/9005 deletion marker are dropped, and
 * duplicate deliveries of the same (target, reactor, emoji) collapse to one.
 */
export function collectReactions(
  events: RelayEvent[] | undefined,
): Map<string, MessageReaction[]> {
  const deletedIds = new Set<string>();
  for (const event of events ?? []) {
    if (
      event.kind !== KIND_DELETION &&
      event.kind !== KIND_NIP29_DELETE_EVENT
    ) {
      continue;
    }
    for (const tag of event.tags) {
      if (tag[0] === "e" && typeof tag[1] === "string") deletedIds.add(tag[1]);
    }
  }

  const byTarget = new Map<string, MessageReaction[]>();
  const seen = new Set<string>();
  for (const event of events ?? []) {
    if (event.kind !== KIND_REACTION || deletedIds.has(event.id)) continue;
    const targetId = reactionTargetId(event.tags);
    if (!targetId || deletedIds.has(targetId)) continue;
    const raw = event.content.trim();
    const emoji = raw === "" || raw === "+" ? "👍" : raw;
    const key = `${targetId}:${event.pubkey}:${emoji}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const reaction: MessageReaction = {
      emoji,
      emojiUrl: reactionEmojiTagUrl(emoji, event.tags),
      pubkey: event.pubkey,
    };
    const bucket = byTarget.get(targetId);
    if (bucket) {
      bucket.push(reaction);
    } else {
      byTarget.set(targetId, [reaction]);
    }
  }
  return byTarget;
}

/**
 * Group one message's reactions into per-emoji chips, in first-reacted order.
 * The first reaction that carries an image URL names the group's URL.
 */
export function groupReactions(
  reactions: MessageReaction[] | undefined,
): ReactionGroup[] {
  const byEmoji = new Map<string, ReactionGroup>();
  for (const { emoji, emojiUrl, pubkey } of reactions ?? []) {
    const group = byEmoji.get(emoji);
    if (group) {
      group.pubkeys.push(pubkey);
      group.emojiUrl ??= emojiUrl;
    } else {
      byEmoji.set(emoji, { emoji, emojiUrl, pubkeys: [pubkey] });
    }
  }
  return [...byEmoji.values()];
}

/**
 * Optimistically toggle `pubkey`'s reaction in a grouped chip list: adds them
 * to (or creates) the emoji's group, or removes them when already present,
 * dropping the group when it empties. Pure — returns a new array.
 */
export function applyReactionToggle(
  groups: ReactionGroup[],
  emoji: string,
  pubkey: string,
  emojiUrl?: string,
): ReactionGroup[] {
  const existing = groups.find((group) => group.emoji === emoji);
  if (!existing) {
    return [...groups, { emoji, emojiUrl, pubkeys: [pubkey] }];
  }
  if (!existing.pubkeys.includes(pubkey)) {
    return groups.map((group) =>
      group === existing
        ? { ...group, pubkeys: [...group.pubkeys, pubkey] }
        : group,
    );
  }
  return groups.flatMap((group) => {
    if (group !== existing) return [group];
    const pubkeys = group.pubkeys.filter((candidate) => candidate !== pubkey);
    return pubkeys.length > 0 ? [{ ...group, pubkeys }] : [];
  });
}
