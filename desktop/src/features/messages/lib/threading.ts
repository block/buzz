import type { RelayEvent } from "@/shared/api/types";

export type ThreadReference = {
  parentId: string | null;
  rootId: string | null;
};

/**
 * Submit-time thread-reply context, captured synchronously by the thread
 * panel before any async sends. `broadcast` opts the reply into the channel
 * timeline (NIP-CW `["broadcast", "1"]`) — Slack's "Also send to #channel".
 */
export type ThreadSendContext = {
  parentEventId: string | null;
  threadHeadId: string | null;
  broadcast?: boolean;
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

export function buildReplyTags(
  channelId: string,
  authorPubkey: string,
  parentEventId: string,
  rootEventId: string,
  mentionPubkeys: string[] = [],
  broadcast = false,
) {
  const tags: string[][] = [
    ["p", authorPubkey],
    ["h", channelId],
  ];

  // Add p-tags for mentioned users so mention-filtered subscriptions
  // (e.g. ACP agent harness) receive the reply event.
  // Best-effort normalization — relay performs authoritative validation.
  for (const pubkey of normalizeMentionPubkeys(mentionPubkeys, authorPubkey)) {
    tags.push(["p", pubkey]);
  }

  // NIP-CW: `["broadcast", "1"]` surfaces the reply on the channel timeline
  // as well as in its thread ("Also send to #channel").
  if (broadcast) {
    tags.push(["broadcast", "1"]);
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
