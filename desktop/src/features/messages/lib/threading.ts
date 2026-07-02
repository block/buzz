import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_FORUM_COMMENT,
  KIND_FORUM_POST,
  KIND_HUDDLE_STARTED,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

// Kinds `resolve_thread_ref` (commands/messages.rs) accepts as reply parents.
// Keep in sync — a kind outside this set must fall back to relay resolution.
const CACHED_REPLY_PARENT_KINDS: readonly number[] = [
  KIND_STREAM_MESSAGE, // 9
  KIND_STREAM_MESSAGE_V2, // 40002
  KIND_FORUM_POST, // 45001
  KIND_FORUM_COMMENT, // 45003
  KIND_HUDDLE_STARTED, // 48100
];

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

export function buildReplyTags(
  channelId: string,
  authorPubkey: string,
  parentEventId: string,
  rootEventId: string,
  mentionPubkeys: string[] = [],
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
 * Resolve the thread root for a reply from the local timeline cache, or
 * `null` when the relay must be consulted.
 *
 * This mirrors `resolve_thread_ref` in `commands/messages.rs` exactly — last
 * `root` marker wins, else last `reply` marker, else the parent itself — so a
 * non-null result is byte-identical to what the Rust side would fetch from
 * the relay. Returns `null` (⇒ caller falls back to relay resolution) when
 * the parent is not cached or its kind is outside the set the Rust resolver
 * queries; note {@link resolveReplyRootId}'s parent-id fallback is NOT safe
 * here, as it would silently mislabel a nested reply as a thread root.
 */
export function resolveCachedReplyRoot(
  parentEventId: string,
  events: RelayEvent[],
): string | null {
  const parent = events.find((event) => event.id === parentEventId);
  if (!parent) {
    return null;
  }
  if (!CACHED_REPLY_PARENT_KINDS.includes(parent.kind)) {
    return null;
  }

  let root: string | null = null;
  let reply: string | null = null;
  for (const tag of parent.tags) {
    if (tag[0] === "e" && typeof tag[1] === "string" && tag.length >= 4) {
      if (tag[3] === "root") {
        root = tag[1];
      } else if (tag[3] === "reply") {
        reply = tag[1];
      }
    }
  }
  const rootHex = root ?? reply;
  return rootHex && rootHex !== parentEventId ? rootHex : parent.id;
}
