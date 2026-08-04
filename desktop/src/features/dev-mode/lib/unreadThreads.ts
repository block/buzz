import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import { getThreadReference } from "@/features/messages/lib/threading";

/**
 * Roots whose thread has replies past the shared read frontier. Built from
 * the same window summaries the transcript renders reply counts from, so
 * the per-card unread dot always agrees with what is on screen.
 */
export function selectUnreadThreadRoots(
  summaries: ReadonlyMap<string, ChannelWindowThreadSummary>,
  getThreadReadAt: (rootId: string) => number | null,
): ReadonlySet<string> {
  const unread = new Set<string>();
  for (const [rootId, summary] of summaries) {
    if (!summary.lastReplyAt) continue;
    const readAt = getThreadReadAt(rootId);
    if (readAt === null || summary.lastReplyAt > readAt) {
      unread.add(rootId);
    }
  }
  return unread;
}

export type UnreadFamilyTarget = {
  channelId: string;
  /**
   * The unread thread needing attention, or null for a top-level unread.
   * Routing opens its side chat only when the unread replies are collapsed
   * in the main view (see useUnreadRouting) — inline unread replies are
   * read just by looking at the channel.
   */
  rootId: string | null;
};

/**
 * Where opening an unread channel family should land: the newest unread
 * thread reply anywhere in the family (main + tabs) wins — that is the
 * agent chat needing attention — falling back to the first family channel
 * (main first) with an unread top-level post. Null means nothing unread,
 * so the caller opens the channel it was asked for.
 *
 * Thread activity is the shared observed-events feed (bounded, newest
 * ~100), already gated to threads relevant to the user; family membership
 * is checked via a Set so hundreds of tabs stay cheap.
 */
export function findUnreadFamilyTarget(input: {
  /** Main channel id first, then its sub-channel (tab) ids. */
  familyChannelIds: readonly string[];
  unreadChannelIds: ReadonlySet<string>;
  topLevelUnreadChannelIds: ReadonlySet<string>;
  threadActivity: readonly {
    channelId: string;
    createdAt: number;
    tags: string[][];
  }[];
  getThreadReadAt: (rootId: string, channelId: string) => number | null;
}): UnreadFamilyTarget | null {
  const family = new Set(input.familyChannelIds);
  let best: { channelId: string; rootId: string; createdAt: number } | null =
    null;
  for (const item of input.threadActivity) {
    if (!family.has(item.channelId)) continue;
    // Channel-level gate keeps routing consistent with the visible dot —
    // the shared model has already folded read markers for cleared channels.
    if (!input.unreadChannelIds.has(item.channelId)) continue;
    const rootId = getThreadReference(item.tags).rootId;
    if (rootId === null) continue;
    const readAt = input.getThreadReadAt(rootId, item.channelId);
    if (readAt !== null && item.createdAt <= readAt) continue;
    if (!best || item.createdAt > best.createdAt) {
      best = { channelId: item.channelId, rootId, createdAt: item.createdAt };
    }
  }
  if (best) {
    return { channelId: best.channelId, rootId: best.rootId };
  }
  for (const channelId of input.familyChannelIds) {
    if (input.topLevelUnreadChannelIds.has(channelId)) {
      return { channelId, rootId: null };
    }
  }
  return null;
}
