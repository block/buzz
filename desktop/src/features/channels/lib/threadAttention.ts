/**
 * Header-level thread attention derivation: the combined "unread or active"
 * list behind the channel header's threads control. Pure functions only —
 * first-seen tracking and React wiring live in useThreadAttentionRows.
 */

export type ThreadAttentionRow = {
  threadHeadId: string;
  /** Resolved display author of the thread head, when loaded. */
  headAuthor: string | null;
  /** Single-line preview of the thread head body, when loaded. */
  headPreview: string | null;
  /** Total replies in the thread (descendants, not just direct children). */
  replyCount: number;
  /** Unread replies in the thread; 0 for active-only rows. */
  unreadCount: number;
  /** Desktop-clock ms when the thread was first seen active; null if idle. */
  activeSince: number | null;
};

/**
 * Uptime at the coarsest useful fidelity — whole seconds, then whole minutes,
 * then whole hours. Never mixes units ("3m", not "3m 12s").
 */
export function formatCoarseUptime(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) return `${totalMinutes}m`;
  return `${Math.floor(totalMinutes / 60)}h`;
}

/** Single-line preview of a message body: collapsed whitespace, hard cap. */
export function buildHeadPreview(body: string): string | null {
  const collapsed = body.replace(/\s+/g, " ").trim();
  if (!collapsed) return null;
  return collapsed.length > 140 ? `${collapsed.slice(0, 140)}…` : collapsed;
}

/**
 * Merge unread counts and active threads into one attention list. Active
 * threads sort first (most recently started on top), then unread threads by
 * reply recency. A thread that is both active and unread appears once, in the
 * active block, carrying its unread count.
 */
export function buildThreadAttentionRows({
  activeSinceByThread,
  getHeadMessage,
  getThreadSummary,
  threadUnreadCounts,
}: {
  activeSinceByThread: ReadonlyMap<string, number>;
  getHeadMessage: (
    threadHeadId: string,
  ) => { author: string; body: string } | undefined;
  getThreadSummary: (
    threadHeadId: string,
  ) => { descendantCount: number; lastReplyAt: number | null } | undefined;
  threadUnreadCounts: ReadonlyMap<string, number>;
}): ThreadAttentionRow[] {
  const ids = new Set<string>(activeSinceByThread.keys());
  for (const [threadHeadId, count] of threadUnreadCounts) {
    if (count > 0) ids.add(threadHeadId);
  }

  const rows = [...ids].map((threadHeadId) => {
    const head = getHeadMessage(threadHeadId);
    const summary = getThreadSummary(threadHeadId);
    return {
      threadHeadId,
      headAuthor: head?.author ?? null,
      headPreview: head ? buildHeadPreview(head.body) : null,
      replyCount: summary?.descendantCount ?? 0,
      unreadCount: threadUnreadCounts.get(threadHeadId) ?? 0,
      activeSince: activeSinceByThread.get(threadHeadId) ?? null,
      lastReplyAt: summary?.lastReplyAt ?? null,
    };
  });

  rows.sort((left, right) => {
    if ((left.activeSince === null) !== (right.activeSince === null)) {
      return left.activeSince === null ? 1 : -1;
    }
    if (left.activeSince !== null && right.activeSince !== null) {
      if (left.activeSince !== right.activeSince) {
        return right.activeSince - left.activeSince;
      }
    }
    const leftRecency = left.lastReplyAt ?? 0;
    const rightRecency = right.lastReplyAt ?? 0;
    if (leftRecency !== rightRecency) return rightRecency - leftRecency;
    return left.threadHeadId.localeCompare(right.threadHeadId);
  });

  return rows.map(({ lastReplyAt: _lastReplyAt, ...row }) => row);
}

/** Field-wise equality for the stabilized rows array (see useStableRows). */
export function areThreadAttentionRowsEqual(
  a: readonly ThreadAttentionRow[],
  b: readonly ThreadAttentionRow[],
): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    const left = a[i];
    const right = b[i];
    if (
      left.threadHeadId !== right.threadHeadId ||
      left.headAuthor !== right.headAuthor ||
      left.headPreview !== right.headPreview ||
      left.replyCount !== right.replyCount ||
      left.unreadCount !== right.unreadCount ||
      left.activeSince !== right.activeSince
    ) {
      return false;
    }
  }
  return true;
}

/** Badge total for the header trigger: unread replies across all threads. */
export function totalUnreadCount(rows: readonly ThreadAttentionRow[]): number {
  let total = 0;
  for (const row of rows) total += row.unreadCount;
  return total;
}
