import * as React from "react";

import {
  areThreadAttentionRowsEqual,
  buildThreadAttentionRows,
  totalUnreadCount,
  type ThreadAttentionRow,
} from "@/features/channels/lib/threadAttention";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import type { TimelineMessage } from "@/features/messages/types";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";

/**
 * Attention rows for the channel header's threads control: every thread that
 * is unread or has an agent actively working in it, one combined list.
 *
 * "Active" is derived from thread-scoped bot typing. The uptime anchor is the
 * first render this hook saw the thread active (a session-local first-seen
 * map, pruned when typing lapses) — the honest floor for a signal that
 * carries no start time of its own. Typing has an 8s TTL, so an agent that
 * goes quiet between tool calls briefly drops out and re-anchors; the coarse
 * Ns/Nm/Nh display keeps that wobble from mattering much.
 */
export function useThreadAttentionRows({
  botTypingEntries,
  threadSummaries,
  threadUnreadCounts,
  timelineMessages,
}: {
  botTypingEntries: TypingIndicatorEntry[];
  threadSummaries: ReadonlyMap<string, ChannelWindowThreadSummary>;
  threadUnreadCounts: ReadonlyMap<string, number>;
  timelineMessages: TimelineMessage[];
}): { rows: readonly ThreadAttentionRow[]; unreadCount: number } {
  // Stable key of active thread heads so the first-seen map only rebuilds
  // when the SET changes — never on unrelated typing-entry reference churn.
  const activeThreadKey = React.useMemo(() => {
    const ids = new Set<string>();
    for (const entry of botTypingEntries) {
      if (entry.threadHeadId !== null) ids.add(entry.threadHeadId);
    }
    return [...ids].sort().join(",");
  }, [botTypingEntries]);

  const firstSeenRef = React.useRef(new Map<string, number>());
  const activeSinceByThread = React.useMemo(() => {
    const idSet = new Set(activeThreadKey ? activeThreadKey.split(",") : []);
    const firstSeen = firstSeenRef.current;
    for (const id of idSet) {
      if (!firstSeen.has(id)) firstSeen.set(id, Date.now());
    }
    for (const id of [...firstSeen.keys()]) {
      if (!idSet.has(id)) firstSeen.delete(id);
    }
    return new Map(firstSeen);
  }, [activeThreadKey]);

  const messageById = React.useMemo(
    () => new Map(timelineMessages.map((message) => [message.id, message])),
    [timelineMessages],
  );

  const rawRows = React.useMemo(
    () =>
      buildThreadAttentionRows({
        activeSinceByThread,
        getHeadMessage: (threadHeadId) => messageById.get(threadHeadId),
        getThreadSummary: (threadHeadId) => threadSummaries.get(threadHeadId),
        threadUnreadCounts,
      }),
    [activeSinceByThread, messageById, threadSummaries, threadUnreadCounts],
  );

  // Field-wise stabilization: busy channels recompute the timeline (and thus
  // rawRows) constantly, but the header memo and menu rows should only see a
  // new reference when a row actually changed.
  const stableRowsRef = React.useRef(rawRows);
  if (
    stableRowsRef.current !== rawRows &&
    !areThreadAttentionRowsEqual(stableRowsRef.current, rawRows)
  ) {
    stableRowsRef.current = rawRows;
  }
  const rows = stableRowsRef.current;

  const unreadCount = React.useMemo(() => totalUnreadCount(rows), [rows]);

  return { rows, unreadCount };
}
