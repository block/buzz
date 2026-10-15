import type { FeedItem } from "@/shared/api/types";

/**
 * Length of the coalescing window, in milliseconds.
 *
 * Long enough to batch a feed-page burst, short enough that a single item still
 * feels immediate. Fixed for this slice — there is deliberately no setting.
 */
export const FEED_COALESCE_WINDOW_MS = 2000;

export type CoalescedFeedSummary = {
  /** Total number of items folded into the single notification. */
  count: number;
  /** Number of distinct senders across those items. */
  agentCount: number;
  /** Items the relay already partitioned as needing the user. */
  needsActionCount: number;
  title: string;
  body: string;
};

function pluralize(count: number, singular: string) {
  return count === 1 ? singular : `${singular}s`;
}

/**
 * Summarize a burst of feed items into ONE notification's title and body.
 *
 * Pure and synchronous: the time window itself is owned by the caller (the feed
 * hook buffers for {@link FEED_COALESCE_WINDOW_MS} and then calls this with
 * whatever accumulated), which keeps this function trivially unit-testable and
 * free of timers.
 *
 * Returns `null` for fewer than two items, which is the signal for the caller to
 * keep using the existing single-item format — coalescing must not regress the
 * common case of one isolated notification.
 *
 * SECURITY: the summary is derived from COUNTS ONLY. No item content, channel
 * name, or sender label reaches the toast, so this path introduces no new
 * untrusted-text surface at all.
 */
export function coalesceFeedItems(
  items: readonly FeedItem[],
): CoalescedFeedSummary | null {
  if (items.length < 2) {
    return null;
  }

  const agentCount = new Set(items.map((item) => item.pubkey)).size;
  const needsActionCount = items.filter(
    (item) => item.category === "needs_action",
  ).length;
  const count = items.length;

  const title = `${agentCount} ${pluralize(agentCount, "agent")}: ${count} ${pluralize(count, "event")}`;
  const body =
    needsActionCount > 0
      ? `${needsActionCount} ${pluralize(needsActionCount, "item")} ${needsActionCount === 1 ? "needs" : "need"} you`
      : `${count} new ${pluralize(count, "update")} in Buzz`;

  return { agentCount, body, count, needsActionCount, title };
}
