/** When the Threads Summarize button should show for a tracked row. */

import type { BestieTrackedThread } from "./bestieThreadTypes";

/**
 * Summarize is available when the thread has never been summarized, or when
 * there has been newer activity (`lastActiveAt`) since `lastSummaryAt`.
 */
export function bestieThreadNeedsSummarize(
  thread: Pick<BestieTrackedThread, "lastActiveAt" | "lastSummaryAt">,
): boolean {
  if (thread.lastSummaryAt == null) return true;
  return thread.lastActiveAt > thread.lastSummaryAt;
}
