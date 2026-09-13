import type { RelayEvent } from "@/shared/api/types";

export type ThreadReference = {
  parentId: string | null;
  rootId: string | null;
};

function getEventTags(tags: string[][]) {
  return tags.filter((tag) => tag[0] === "e" && typeof tag[1] === "string");
}

export function getChannelIdFromTags(tags: string[][]) {
  return tags.find((tag) => tag[0] === "h")?.[1] ?? null;
}

export function isBroadcastReply(tags: string[][]): boolean {
  return tags.some((tag) => tag[0] === "broadcast" && tag[1] === "1");
}

export function isThreadReply(tags: string[][]): boolean {
  const ref = getThreadReference(tags);
  return ref.parentId !== null && !isBroadcastReply(tags);
}

export function getThreadReference(tags: string[][]): ThreadReference {
  const eventTags = getEventTags(tags);

  if (eventTags.length === 0) {
    return {
      parentId: null,
      rootId: null,
    };
  }

  const rootTag = eventTags.find((tag) => tag[3] === "root");
  const replyTag =
    [...eventTags].reverse().find((tag) => tag[3] === "reply") ?? null;

  if (!replyTag) {
    return {
      parentId: null,
      rootId: null,
    };
  }

  const parentId = replyTag[1] ?? null;

  return {
    parentId,
    rootId: rootTag?.[1] ?? parentId,
  };
}

/**
 * Best-effort client-side normalization of mention pubkeys: lowercase, deduplicate, skip self.
 * The relay performs authoritative validation (hex format, 64-char length, cap of 50)
 * on top of the same normalization — this helper keeps optimistic UI tags consistent.
 */
export function normalizeMentionPubkeys(
  mentionPubkeys: string[],
  selfPubkey: string,
): string[] {
  const selfLower = selfPubkey.toLowerCase();
  const seen = new Set<string>([selfLower]);
  const result: string[] = [];
  for (const pk of mentionPubkeys) {
    const lower = pk.toLowerCase();
    if (seen.has(lower)) {
      continue;
    }
    seen.add(lower);
    result.push(lower);
  }
  return result;
}

/**
 * Mentions an edit *newly adds*, relative to the original message body.
 *
 * The composer resolves both bodies to pubkey lists with the same
 * channel-roster resolver the send path uses, then hands them here. We
 * normalize the edited body's set (lowercase / dedup / drop self) and keep
 * only pubkeys that were not already present in the original body — compared
 * case-insensitively so a case-only difference is never treated as "new".
 *
 * A typo-fix edit that leaves the mention set unchanged yields `[]`, so the
 * edit event carries no `p` tags and re-wakes nobody. Only genuinely new
 * mentions get notified.
 */
export function diffAddedMentionPubkeys(
  originalPubkeys: string[],
  editedPubkeys: string[],
  selfPubkey: string,
): string[] {
  const original = new Set(originalPubkeys.map((pk) => pk.toLowerCase()));
  return normalizeMentionPubkeys(editedPubkeys, selfPubkey).filter(
    (pubkey) => !original.has(pubkey),
  );
}

export function buildReplyTags(
  channelId: string,
  authorPubkey: string,
  parentEventId: string,
  rootEventId: string,
  mentionPubkeys: string[] = [],
  replyTargetAuthorPubkey?: string,
) {
  const tags: string[][] = [
    ["p", authorPubkey],
    ["h", channelId],
  ];

  // Add p-tags for mentioned users so mention-filtered subscriptions
  // (e.g. ACP agent harness) receive the reply event.
  // Best-effort normalization — relay performs authoritative validation.
  // The replied-to event's author is folded in too: replying to a message is
  // itself an implicit way of addressing its author (notably agent accounts,
  // which wake on an explicit @mention / p-tag), so include it even when the
  // composer text contains no literal "@mention".
  const mentionSources = replyTargetAuthorPubkey
    ? [...mentionPubkeys, replyTargetAuthorPubkey]
    : mentionPubkeys;
  for (const pubkey of normalizeMentionPubkeys(mentionSources, authorPubkey)) {
    tags.push(["p", pubkey]);
  }

  if (parentEventId === rootEventId) {
    tags.push(["e", rootEventId, "", "reply"]);
    return tags;
  }

  tags.push(["e", rootEventId, "", "root"]);
  tags.push(["e", parentEventId, "", "reply"]);
  return tags;
}

export function buildThreadReferenceTags(
  channelId: string,
  parentEventId: string | null,
  rootEventId: string | null,
) {
  const tags: string[][] = [["h", channelId]];

  if (!parentEventId) {
    return tags;
  }

  if (!rootEventId || parentEventId === rootEventId) {
    tags.push(["e", parentEventId, "", "reply"]);
    return tags;
  }

  tags.push(["e", rootEventId, "", "root"]);
  tags.push(["e", parentEventId, "", "reply"]);
  return tags;
}

export function resolveReplyRootId(
  parentEventId: string,
  events: RelayEvent[],
) {
  const parent = events.find((event) => event.id === parentEventId);
  if (!parent) {
    return parentEventId;
  }

  const thread = getThreadReference(parent.tags);
  return thread.rootId ?? parent.id;
}

/**
 * Resolve the author pubkey of the event being replied to, if it is present in
 * the local message cache.
 *
 * Mirrors `resolveReplyRootId`'s lookup (same event array, same `id` match) so
 * callers can fold the replied-to author into the reply's `p` tags — replying
 * to a message implicitly addresses its author (e.g. an agent that wakes on an
 * explicit mentioning `p` tag), even when the composer body has no literal
 * `@mention`. Returns `null` when the parent event isn't in the cache yet.
 */
export function resolveReplyTargetAuthorPubkey(
  parentEventId: string,
  events: RelayEvent[],
): string | null {
  return events.find((event) => event.id === parentEventId)?.pubkey ?? null;
}
