import { isAttentionWorthy } from "@/features/attention/lib/attention";
import type { InboxItem } from "@/features/home/lib/inbox";
import { summaryLineFor } from "@/features/home/lib/summaryLine";
import { getThreadReference } from "@/features/messages/lib/threading";

export const CATCH_UP_LINES_PER_CHANNEL = 10;

export type CatchUpLine = {
  /** Message event id — the deep-link target. */
  id: string;
  channelId: string;
  threadRootId: string | null;
  authorPubkey: string;
  createdAt: number;
  summary: string;
};

export type CatchUpChannelGroup = {
  channelId: string;
  channelLabel: string;
  latestActivityAt: number;
  /** Oldest first (reads as narrative), capped at CATCH_UP_LINES_PER_CHANNEL. */
  lines: CatchUpLine[];
  /** Known unread lines beyond the cap — "N more in #channel". */
  moreCount: number;
};

export type CatchUpDigest = {
  /** Ordered by most recent activity. Channels with nothing unread omitted. */
  groups: CatchUpChannelGroup[];
  /** Unread items that qualify as attention asks — counted, never listed. */
  needsYouCount: number;
  totalLineCount: number;
};

/**
 * Project the Inbox's unread state into a channel-grouped reading.
 *
 * Works entirely from what the Inbox already has in cache: the feed's
 * grouped conversation items and the shared NIP-RS read boundary
 * (`doneSet` marks fully-read conversations; `getItemReadAt` yields the
 * per-conversation marker for the per-message cut). No new queries.
 */
export function buildCatchUpDigest(options: {
  items: InboxItem[];
  doneSet: ReadonlySet<string>;
  getItemReadAt: (item: InboxItem) => number | null;
}): CatchUpDigest {
  const { items, doneSet, getItemReadAt } = options;
  const buckets = new Map<
    string,
    { channelLabel: string; lines: Map<string, CatchUpLine> }
  >();
  let needsYouCount = 0;

  for (const item of items) {
    if (doneSet.has(item.id)) {
      continue;
    }
    // Dedup vs Attention: asks live in the Attention surface; Catch up only
    // counts them so nothing is read twice.
    if (isAttentionWorthy(item)) {
      needsYouCount++;
      continue;
    }
    const channelId = item.item.channelId;
    if (!channelId) {
      continue;
    }

    const readAt = getItemReadAt(item);
    const unreadMessages = item.groupItems.filter(
      (message) => readAt === null || message.createdAt > readAt,
    );
    // The conversation is unread but every cached message predates the
    // marker (partial cache) — fall back to the representative message.
    const sourceMessages =
      unreadMessages.length > 0 ? unreadMessages : [item.item];

    const bucket = buckets.get(channelId) ?? {
      channelLabel: item.channelLabel ?? item.item.channelName ?? "channel",
      lines: new Map<string, CatchUpLine>(),
    };
    for (const message of sourceMessages) {
      if (bucket.lines.has(message.id)) {
        continue;
      }
      bucket.lines.set(message.id, {
        id: message.id,
        channelId,
        threadRootId: getThreadReference(message.tags).rootId,
        authorPubkey: message.pubkey,
        createdAt: message.createdAt,
        summary: summaryLineFor(message.content) || item.preview,
      });
    }
    buckets.set(channelId, bucket);
  }

  const groups: CatchUpChannelGroup[] = [];
  let totalLineCount = 0;
  for (const [channelId, bucket] of buckets) {
    const lines = [...bucket.lines.values()].sort(
      (a, b) => a.createdAt - b.createdAt,
    );
    if (lines.length === 0) {
      continue;
    }
    totalLineCount += lines.length;
    groups.push({
      channelId,
      channelLabel: bucket.channelLabel,
      latestActivityAt: lines[lines.length - 1].createdAt,
      lines: lines.slice(0, CATCH_UP_LINES_PER_CHANNEL),
      moreCount: Math.max(0, lines.length - CATCH_UP_LINES_PER_CHANNEL),
    });
  }
  groups.sort((a, b) => b.latestActivityAt - a.latestActivityAt);

  return { groups, needsYouCount, totalLineCount };
}
