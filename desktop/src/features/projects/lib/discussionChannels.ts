/**
 * "Discussed in" channel discovery for Buzz git entities.
 *
 * Chat messages that reference a PR, issue, or repository do so only through
 * `buzz://` links in their *content* — they carry no entity tags (see
 * `useMessageLinkPreviews.ts`). So discovery runs the relay's NIP-50
 * full-text search over message content and groups the hits by channel.
 *
 * Query construction leans on the FTS tokenizer: the relay ANDs every token
 * of the search text, and `buzz://` links tokenize into their query-param
 * values. A PR/issue link contains the entity's 64-hex event id (globally
 * unique token), and every repo/PR/issue link contains the repository
 * coordinate's `owner` pubkey and `d`-tag — so searching those tokens finds
 * exactly the messages linking the entity, across all channels the viewer
 * can read (the relay re-authorizes each hit).
 */

import type { SearchHit } from "@/shared/api/searchTypes";

export type DiscussionChannel = {
  id: string;
  /** Channel display name from the search hit; null when the relay omitted it. */
  name: string | null;
  messageCount: number;
  /** Unix seconds of the newest matching message. */
  lastActivityAt: number;
  /** Unique author pubkeys, most recent speaker first. */
  participants: string[];
};

/**
 * Search text matching messages that link a specific PR or issue: the event
 * id is a single 64-hex token unique to the entity, present in every
 * `buzz://pr|issue?id=…` link.
 */
export function entityDiscussionQuery(eventId: string): string {
  return eventId;
}

/** Channel the entity was created from (`h` tag, author-claimed). The tag
 * proves only the channel — no event ties any specific message to the
 * entity, so the origin renders as a channel-only row: never fetch nearby
 * channel traffic to fabricate, quote, or attribute a "spawning"
 * conversation. */
export type DiscussionOrigin = {
  channelId: string;
  createdAt: number;
  pubkey: string;
};

/** Prepend the origin channel when search did not already return it. */
export function mergeOriginDiscussionChannel(
  channels: DiscussionChannel[],
  origin: DiscussionOrigin | null | undefined,
): DiscussionChannel[] {
  const channelId = origin?.channelId?.trim();
  if (!channelId || !origin) return channels;
  if (channels.some((channel) => channel.id === channelId)) {
    return channels;
  }
  return [
    {
      id: channelId,
      name: null,
      messageCount: 1,
      lastActivityAt: origin.createdAt,
      participants: [origin.pubkey.toLowerCase()],
    },
    ...channels,
  ];
}

/**
 * Search text matching messages that link a repository or any of its PRs
 * and issues: all those links carry `owner=<pubkey>&d=<dtag>`, so the owner
 * pubkey and d-tag tokens together identify the repository coordinate.
 */
export function repositoryDiscussionQuery(repository: {
  owner: string;
  dtag: string;
}): string {
  return `${repository.owner} ${repository.dtag}`;
}

/**
 * Chat cites commits by either the full or the abbreviated hash, so match
 * both. `websearch_to_tsquery` (the relay's NIP-50 parser) treats a literal
 * `OR` between words as a disjunction.
 */
export function commitDiscussionQuery(commit: {
  hash: string;
  shortHash?: string | null;
}): string {
  const short = commit.shortHash ?? commit.hash.slice(0, 7);
  if (!short || short === commit.hash) {
    return commit.hash;
  }
  return `${commit.hash} OR ${short}`;
}

/**
 * Group search hits into unique channels, ordered by message count then
 * recency. Channel-less hits (no `h` tag) are dropped.
 */
export function groupDiscussionChannels(
  hits: readonly Pick<
    SearchHit,
    "channelId" | "channelName" | "createdAt" | "pubkey"
  >[],
): DiscussionChannel[] {
  const byChannel = new Map<string, DiscussionChannel>();
  // Newest first so each channel's participant list leads with the most
  // recent speaker.
  const ordered = [...hits].sort((a, b) => b.createdAt - a.createdAt);
  for (const hit of ordered) {
    if (!hit.channelId) continue;
    const pubkey = hit.pubkey.toLowerCase();
    const existing = byChannel.get(hit.channelId);
    if (existing) {
      existing.messageCount += 1;
      existing.lastActivityAt = Math.max(
        existing.lastActivityAt,
        hit.createdAt,
      );
      if (existing.name === null && hit.channelName) {
        existing.name = hit.channelName;
      }
      if (!existing.participants.includes(pubkey)) {
        existing.participants.push(pubkey);
      }
    } else {
      byChannel.set(hit.channelId, {
        id: hit.channelId,
        name: hit.channelName ?? null,
        messageCount: 1,
        lastActivityAt: hit.createdAt,
        participants: [pubkey],
      });
    }
  }
  return [...byChannel.values()].sort(
    (a, b) =>
      b.messageCount - a.messageCount || b.lastActivityAt - a.lastActivityAt,
  );
}

/**
 * Human list of discussing names: "Alice", "Alice and Bob",
 * "Alice, Bob and Carol", "Alice, Bob and 3 others".
 */
export function formatNameList(names: readonly string[], maxNames = 3): string {
  if (names.length === 0) return "";
  if (names.length === 1) return names[0];
  if (names.length <= maxNames) {
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  }
  const shown = names.slice(0, maxNames - 1);
  const others = names.length - shown.length;
  return `${shown.join(", ")} and ${others} others`;
}

// Generous cap: the row's CSS `truncate` does the visual cut at the card
// edge, so this only bounds DOM size against very long messages. Keep it
// comfortably above what an ultrawide screen can show on one line.
const SNIPPET_MAX_CHARS = 400;

/**
 * One-line preview of a discussing message: entity links and coordinates are
 * dropped (the reader is already looking at the entity), whitespace collapses,
 * and long content truncates on an ellipsis.
 */
export function discussionSnippet(content: string): string {
  const cleaned = content
    .replace(/buzz:\/\/\S+/g, "")
    .replace(/\b\d{5}:[0-9a-f]{64}:\S+/gi, "")
    .replace(/\s+/g, " ")
    .trim();
  if (cleaned.length === 0) {
    return "Shared a link to this.";
  }
  if (cleaned.length <= SNIPPET_MAX_CHARS) {
    return cleaned;
  }
  return `${cleaned.slice(0, SNIPPET_MAX_CHARS - 1).trimEnd()}…`;
}
