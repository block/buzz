import * as React from "react";

import {
  buildCreatedAtByMessageId,
  buildDirectRepliesByParentId,
  buildDirectReplyIdsByParentId,
  collectReplyDescendantIds,
  subtreeMaxCreatedAt,
} from "@/features/channels/lib/subtreeCreatedAt";
import { computeThreadReplyUnreadCounts } from "@/features/channels/lib/threadReplyUnreadCounts";
import { computeThreadBadgeCounts } from "@/features/channels/lib/threadBadgeCounts";
import { seedThreadBadgeFrontiers } from "@/features/channels/lib/threadBadgeFrontier";
import {
  buildThreadPanelDataFromIndex,
  buildThreadPanelIndex,
} from "@/features/messages/lib/threadPanel";
import {
  computeChannelUnreadMarker,
  computeThreadUnreadMarker,
} from "@/features/messages/lib/unreadMarker";
import type { TimelineMessage } from "@/features/messages/types";
import { isConversationalUnreadKind } from "@/shared/constants/kinds";

import { useWelcomeInitialUnreadSuppression } from "./useWelcomeInitialUnreadSuppression";

type UseChannelUnreadStateOptions = {
  activeChannelId: string | null;
  timelineMessages: TimelineMessage[];
  currentPubkey: string | undefined;
  openThreadHeadId: string | null;
  threadReplyTargetId: string | null;
  expandedThreadReplyIds: ReadonlySet<string>;
  isNotifiedForCurrentThread: boolean;
  getChannelReadAt: (channelId: string) => number | null;
  getThreadReadAt: (rootId: string) => number | null;
  markChannelUnread: (channelId: string) => void;
  markThreadRead: (rootId: string, timestamp: number) => void;
  isNotifiedForThread: (rootId: string) => boolean;
  readStateVersion: number;
};

/**
 * All read-state derivation for an active channel that is computed from the
 * formatted timeline: the open-time read frontiers (channel / thread / per-row
 * thread badges), the thread-panel projection, and every unread marker and
 * unread-count the channel surface renders.
 *
 * Extracted from ChannelScreen so the screen stays under the file-size cap and
 * the NIP-RS read-state machinery lives as one cohesive unit. Behavior is
 * unchanged — the only inputs are the formatted timeline plus the AppShell
 * read-state accessors, and the hook owns the refs/effects that snapshot the
 * "what was unread on open" frontiers.
 */
export function useChannelUnreadState({
  activeChannelId,
  timelineMessages,
  currentPubkey,
  openThreadHeadId,
  threadReplyTargetId,
  expandedThreadReplyIds,
  isNotifiedForCurrentThread,
  getChannelReadAt,
  getThreadReadAt,
  markChannelUnread,
  markThreadRead,
  isNotifiedForThread,
  readStateVersion,
}: UseChannelUnreadStateOptions) {
  // Capture the read frontier as it stood the instant this channel was opened,
  // BEFORE the mark-read effect (in ChannelScreen) advances it to latest.
  // Written during render (not in an effect) so the value is read prior to any
  // effect for this commit — the divider must reflect "what was unread on
  // open", not the post-open frontier. Keyed per channel and recomputed only
  // when the channel id changes, never when the frontier advances, or the
  // divider would vanish the moment the open marks the channel read.
  const openFrontierRef = React.useRef(new Map<string, number | null>());
  if (activeChannelId && !openFrontierRef.current.has(activeChannelId)) {
    openFrontierRef.current.set(
      activeChannelId,
      getChannelReadAt(activeChannelId),
    );
  }
  const openFrontierSeconds = activeChannelId
    ? (openFrontierRef.current.get(activeChannelId) ?? null)
    : null;
  // Channels the user manually marked unread this session. A deliberate
  // mark-unread has no meaningful "new" boundary inside the timeline — the
  // open-time snapshot already covers every message — so the pill and divider
  // would otherwise render nothing while the sidebar dot says unread. Suppress
  // the marker for such channels to avoid that visible contradiction. The flag
  // is cleared on re-open (a fresh snapshot is recomputed for the channel).
  const forcedUnreadRef = React.useRef(new Set<string>());
  const [, forceUnreadRender] = React.useReducer((n: number) => n + 1, 0);
  const isActiveChannelForcedUnread =
    !!activeChannelId && forcedUnreadRef.current.has(activeChannelId);
  const isActiveWelcomeInitialUnreadSuppressed =
    useWelcomeInitialUnreadSuppression(activeChannelId, forceUnreadRender);
  // Drop the forced-unread flag when the user leaves a channel, so reopening
  // it recomputes a normal marker rather than staying suppressed forever.
  React.useEffect(() => {
    const channelId = activeChannelId;
    if (!channelId) return;
    return () => {
      forcedUnreadRef.current.delete(channelId);
    };
  }, [activeChannelId]);
  // Clear the open-time frontier on channel leave so re-visiting captures a
  // fresh read position. Without this, switching away and back would reuse the
  // stale frontier from the first open, producing a phantom "New" divider over
  // already-read messages.
  React.useEffect(() => {
    const channelId = activeChannelId;
    if (!channelId) return;
    return () => {
      openFrontierRef.current.delete(channelId);
    };
  }, [activeChannelId]);

  const directReplyIdsByParentId = React.useMemo(
    () => buildDirectReplyIdsByParentId(timelineMessages),
    [timelineMessages],
  );
  const directRepliesByParentId = React.useMemo(
    () => buildDirectRepliesByParentId(timelineMessages),
    [timelineMessages],
  );
  const getFirstReplyIdForMessage = React.useCallback(
    (messageId: string) => directReplyIdsByParentId.get(messageId)?.[0] ?? null,
    [directReplyIdsByParentId],
  );
  const getReplyDescendantIdsForMessage = React.useCallback(
    (messageId: string) =>
      collectReplyDescendantIds(messageId, directReplyIdsByParentId),
    [directReplyIdsByParentId],
  );
  const createdAtByMessageId = React.useMemo(
    () => buildCreatedAtByMessageId(timelineMessages),
    [timelineMessages],
  );
  // Newest createdAt across an expanded branch (the message itself plus every
  // descendant). Drilling into a branch advances the thread frontier to this,
  // consuming everything chronologically up to the deepest reply read. Returns
  // null when the message is absent so the caller skips the read-state write.
  const getSubtreeMaxCreatedAt = React.useCallback(
    (messageId: string) =>
      subtreeMaxCreatedAt(
        messageId,
        directReplyIdsByParentId,
        createdAtByMessageId,
      ),
    [createdAtByMessageId, directReplyIdsByParentId],
  );
  const threadPanelIndex = React.useMemo(
    () => buildThreadPanelIndex(timelineMessages),
    [timelineMessages],
  );
  const threadPanelData = React.useMemo(
    () =>
      buildThreadPanelDataFromIndex(
        threadPanelIndex,
        openThreadHeadId,
        threadReplyTargetId,
        expandedThreadReplyIds,
      ),
    [
      expandedThreadReplyIds,
      openThreadHeadId,
      threadReplyTargetId,
      threadPanelIndex,
    ],
  );
  const openThreadHeadMessage = threadPanelData.threadHead;
  const threadMessages = threadPanelData.visibleReplies;
  const threadReplyTargetMessage = threadPanelData.replyTargetMessage;

  // Oldest unread top-level message + count from the open-time frontier.
  // Keyed per channel so the pill/divider survive the mark-read effect.
  // Non-conversational kinds (system rows, job-lifecycle events) are filtered
  // out first so they don't inflate the pill; see isConversationalUnreadKind.
  const { firstUnreadMessageId, unreadCount } = React.useMemo(
    () =>
      computeChannelUnreadMarker(
        timelineMessages.filter((message) =>
          isConversationalUnreadKind(message.kind),
        ),
        openFrontierSeconds,
        isActiveChannelForcedUnread || isActiveWelcomeInitialUnreadSuppressed,
        currentPubkey,
      ),
    [
      currentPubkey,
      isActiveChannelForcedUnread,
      isActiveWelcomeInitialUnreadSuppressed,
      openFrontierSeconds,
      timelineMessages,
    ],
  );

  // --- Thread unread state ---
  // Capture the thread read frontier on open (same pattern as channel frontier).
  // Keyed per thread root so switching threads captures a fresh frontier.
  const threadOpenFrontierRef = React.useRef(new Map<string, number | null>());
  if (
    openThreadHeadId &&
    !threadOpenFrontierRef.current.has(openThreadHeadId)
  ) {
    threadOpenFrontierRef.current.set(
      openThreadHeadId,
      getThreadReadAt(openThreadHeadId),
    );
  }
  const threadOpenFrontierSeconds = openThreadHeadId
    ? (threadOpenFrontierRef.current.get(openThreadHeadId) ?? null)
    : null;
  // Clear the thread frontier when the thread closes so re-opening captures fresh.
  React.useEffect(() => {
    const rootId = openThreadHeadId;
    if (!rootId) return;
    return () => {
      threadOpenFrontierRef.current.delete(rootId);
    };
  }, [openThreadHeadId]);
  // Mark thread read when the panel opens, advancing the frontier to the max
  // createdAt over the head and its ENTIRE subtree — every reply, including
  // ones nested in collapsed branches. Opening a notified thread means engaging
  // with it, so the badge must collapse the instant the panel opens (not wait
  // for a channel change or for each branch to be expanded). The badge counts
  // the whole subtree (computeThreadBadgeCounts), so marking only the visible
  // direct replies would leave it lit whenever the unread lives in a nested
  // reply — the reported bug. Consuming collapsed branches here is not lossy:
  // a NEWER reply re-raises the badge, because the unread comparison is strictly
  // `createdAt > frontier` (computeThreadUnreadMarker) and the badge snapshot
  // advances toward the live marker (nextThreadBadgeFrontier).
  // Only persist read state for threads the user has notification interest in
  // (participated, authored, or followed) to avoid bloating the context blob.
  React.useEffect(() => {
    if (!openThreadHeadId) return;
    if (!isNotifiedForCurrentThread) return;
    const openReadCeiling = getSubtreeMaxCreatedAt(openThreadHeadId);
    if (openReadCeiling === null) return;
    markThreadRead(openThreadHeadId, openReadCeiling);
  }, [
    openThreadHeadId,
    getSubtreeMaxCreatedAt,
    markThreadRead,
    isNotifiedForCurrentThread,
  ]);
  // Compute the in-thread "New" divider position from the open-time frontier.
  const { firstUnreadReplyId: threadFirstUnreadReplyId } = React.useMemo(() => {
    if (!openThreadHeadId || threadMessages.length === 0) {
      return { firstUnreadReplyId: null, unreadCount: 0 };
    }
    const replies = threadMessages.map((entry) => entry.message);
    return computeThreadUnreadMarker(
      replies,
      threadOpenFrontierSeconds,
      currentPubkey,
    );
  }, [
    currentPubkey,
    openThreadHeadId,
    threadMessages,
    threadOpenFrontierSeconds,
  ]);
  // Per-row subtree unread counts for the in-panel thread summary rows. Scoped
  // to the open thread's subtree and measured against the open-time frontier
  // snapshot (threadOpenFrontierSeconds) — the same boundary the in-thread
  // divider uses (above). The LIVE root marker can't be used here: on
  // channel-open markChannelRead advances the channel marker to the newest
  // top-level message, and effective(thread) = max(thread_own, channel_marker),
  // so a channel marker past the nested replies would zero every badge the
  // instant the panel opens. The snapshot reflects "what was unread on open."
  // Expand-clears-badge is preserved independently: it's driven by the
  // expandedSubtreeReplyIds gate inside computeThreadReplyUnreadCounts, not by
  // the frontier.
  const threadReplyUnreadCounts = React.useMemo(
    () =>
      openThreadHeadId
        ? computeThreadReplyUnreadCounts({
            timelineMessages,
            subtreeReplyIds: getReplyDescendantIdsForMessage(openThreadHeadId),
            visibleReplyIds: threadMessages.map((entry) => entry.message.id),
            expandedReplyIds: expandedThreadReplyIds,
            expandedSubtreeReplyIds: new Set(
              [...expandedThreadReplyIds].flatMap((id) =>
                getReplyDescendantIdsForMessage(id),
              ),
            ),
            frontierSeconds: threadOpenFrontierSeconds,
            currentPubkey,
          })
        : new Map<string, number>(),
    [
      openThreadHeadId,
      threadMessages,
      timelineMessages,
      threadOpenFrontierSeconds,
      expandedThreadReplyIds,
      getReplyDescendantIdsForMessage,
      currentPubkey,
    ],
  );
  // Snapshot per-thread read frontiers at channel-open time. Same pattern as
  // openFrontierRef: captured during render (before the mark-read effect) so
  // the badge reflects "what was unread on open" rather than the post-advance
  // frontier. Keyed by activeChannelId → rootId → frontier value.
  const threadBadgeFrontiersRef = React.useRef(
    new Map<string, Map<string, number | null>>(),
  );
  if (activeChannelId) {
    let channelFrontiers = threadBadgeFrontiersRef.current.get(activeChannelId);
    if (!channelFrontiers) {
      channelFrontiers = new Map();
      threadBadgeFrontiersRef.current.set(activeChannelId, channelFrontiers);
    }
    seedThreadBadgeFrontiers(
      channelFrontiers,
      timelineMessages,
      directRepliesByParentId,
      isNotifiedForThread,
      getThreadReadAt,
    );
  }
  // Clear the thread badge frontiers on channel leave (same cleanup as
  // openFrontierRef) so re-visiting captures fresh snapshots.
  React.useEffect(() => {
    const channelId = activeChannelId;
    if (!channelId) return;
    return () => {
      threadBadgeFrontiersRef.current.delete(channelId);
    };
  }, [activeChannelId]);
  // Per-thread unread counts for the main-timeline summary rows. Pure logic
  // lives in computeThreadBadgeCounts; readStateVersion is an intentional
  // recompute trigger so the badge re-reads the snapshot the seed block above
  // advanced toward the live marker on mark-read.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is the intentional recompute trigger
  const threadUnreadCounts = React.useMemo(
    () =>
      computeThreadBadgeCounts(
        timelineMessages,
        directRepliesByParentId,
        activeChannelId
          ? threadBadgeFrontiersRef.current.get(activeChannelId)
          : undefined,
        isNotifiedForThread,
        currentPubkey,
      ),
    [
      activeChannelId,
      currentPubkey,
      timelineMessages,
      directRepliesByParentId,
      isNotifiedForThread,
      readStateVersion,
    ],
  );

  const handleMarkUnread = React.useCallback(() => {
    if (!activeChannelId) return;
    // Mirror the deliberate mark-unread locally so the timeline marker is
    // suppressed (see forcedUnreadRef above). Re-render so the memo re-runs.
    forcedUnreadRef.current.add(activeChannelId);
    forceUnreadRender();
    markChannelUnread(activeChannelId);
  }, [activeChannelId, markChannelUnread]);

  return {
    createdAtByMessageId,
    directReplyIdsByParentId,
    firstUnreadMessageId,
    getFirstReplyIdForMessage,
    getReplyDescendantIdsForMessage,
    getSubtreeMaxCreatedAt,
    handleMarkUnread,
    openThreadHeadMessage,
    threadFirstUnreadReplyId,
    threadMessages,
    threadReplyTargetMessage,
    threadReplyUnreadCounts,
    threadUnreadCounts,
    unreadCount,
  };
}
