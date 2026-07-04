import * as React from "react";

import {
  buildCreatedAtByMessageId,
  buildDirectReplyIdsByParentId,
  buildRepliesByRootId,
  collectReplyDescendantIds,
} from "@/features/channels/lib/subtreeCreatedAt";
import { computeThreadReplyUnreadCounts } from "@/features/channels/lib/threadReplyUnreadCounts";
import { computeThreadBadgeCounts } from "@/features/channels/lib/threadBadgeCounts";
import {
  useStableArrayShallow,
  useStableMap,
} from "@/shared/hooks/useStableReference";
import {
  buildThreadPanelDataFromIndex,
  buildThreadPanelIndex,
  type MainTimelineEntry,
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
  openThreadMessages?: MainTimelineEntry[];
  getChannelReadAt: (channelId: string) => number | null;
  getMessageReadAt: (messageId: string) => number | null;
  markChannelUnread: (channelId: string) => void;
  markMessageRead: (messageId: string, timestamp: number) => void;
  isThreadMuted: (rootId: string) => boolean;
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
  openThreadMessages,
  getChannelReadAt,
  getMessageReadAt,
  markChannelUnread,
  markMessageRead,
  isThreadMuted,
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
  const [forcedUnreadVersion, forceUnreadRender] = React.useReducer(
    (n: number) => n + 1,
    0,
  );
  // Per-message analog of forcedUnreadRef (LP4 v3 mark-unread). A monotonic
  // grow-only msg:<id> marker cannot move the read-line backward, so a
  // deliberate mark-unread lives in this session-local set, read ONLY as an
  // OR-overlay by the badge predicates below — never written to the marker
  // store. Cleared on channel-leave (same lifecycle as the channel set), so
  // it does not survive reload, exactly like channel mark-unread today.
  const forcedUnreadMsgRef = React.useRef(new Set<string>());
  const isMsgForcedUnread = React.useCallback(
    (messageId: string) => forcedUnreadMsgRef.current.has(messageId),
    [],
  );
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
      // Clear per-message forced-unread too: switching channels ends the
      // session window for both the channel-level and message-level overlays.
      forcedUnreadMsgRef.current.clear();
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
  const repliesByRootId = React.useMemo(
    () => buildRepliesByRootId(timelineMessages),
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
  const messageById = React.useMemo(
    () => new Map(timelineMessages.map((message) => [message.id, message])),
    [timelineMessages],
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
  const threadMessages = useStableArrayShallow(
    openThreadMessages ?? threadPanelData.visibleReplies,
  );
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
  // Snapshot the per-message read state for the open thread's visible replies
  // the instant the thread opens, BEFORE the on-open mark-read effect advances
  // those markers. This anchors the in-thread "New" divider to "what was unread
  // when I opened this thread" — the exact thread-level analog of the channel
  // divider's openFrontierRef. Read ONLY by the divider below; the badge
  // predicates read effective(msg:<id>) live, so this snapshot is a separate
  // concern (divider position) from the badge read-line — not a second source
  // of truth for the same read-line. Keyed per thread root so switching threads
  // captures a fresh snapshot; cleared on close so re-opening re-snapshots.
  const threadOpenReadSnapshotRef = React.useRef(
    new Map<string, Map<string, number | null>>(),
  );
  // Record a reply's read state into the open thread's divider snapshot the
  // first time we observe it, before any marker advance. Idempotent per reply
  // (the first capture wins), so a value taken before a mark-read is never
  // overwritten by the post-mark value. Keyed to the current open thread so a
  // stale entry from a previous open cannot leak across a close→reopen cycle
  // (the snapshot is dropped on close by the effect below).
  const captureDividerReadState = React.useCallback(
    (replyId: string) => {
      if (!openThreadHeadId) return;
      let snapshot = threadOpenReadSnapshotRef.current.get(openThreadHeadId);
      if (!snapshot) {
        snapshot = new Map<string, number | null>();
        threadOpenReadSnapshotRef.current.set(openThreadHeadId, snapshot);
      }
      if (!snapshot.has(replyId)) {
        snapshot.set(replyId, getMessageReadAt(replyId));
      }
    },
    [getMessageReadAt, openThreadHeadId],
  );
  if (openThreadHeadId) {
    // Capture each visible reply's read state the first render it appears —
    // before the on-open mark-read effect advances its marker. Replies revealed
    // by expanding a branch are captured eagerly in markRevealedRepliesRead
    // (before that path's synchronous mark-read), so this render-time pass
    // covers replies present at open and acts as the fallback for any reply
    // that reaches render without being pre-captured.
    for (const entry of threadMessages) {
      captureDividerReadState(entry.message.id);
    }
  }
  React.useEffect(() => {
    const rootId = openThreadHeadId;
    if (!rootId) return;
    return () => {
      threadOpenReadSnapshotRef.current.delete(rootId);
    };
  }, [openThreadHeadId]);
  // Mark the revealed set read when the thread opens (LP4 v3): only the replies
  // visible on open are read, never the whole subtree. A reply nested in a
  // still-collapsed branch keeps its badge until it too is revealed (the
  // deliberate reversal of #1118's whole-subtree-on-open). Each revealed reply
  // gets its own msg:<id> marker advanced to its createdAt; a NEWER reply
  // re-raises the badge because the predicate is strictly createdAt > read.
  React.useEffect(() => {
    if (!openThreadHeadId) return;
    if (isThreadMuted(openThreadHeadId)) return;
    for (const entry of threadMessages) {
      markMessageRead(entry.message.id, entry.message.createdAt);
    }
  }, [openThreadHeadId, threadMessages, markMessageRead, isThreadMuted]);
  // In-thread "New" divider position. Reads the open-time snapshot (frozen
  // before the mark-read effect above), so the divider does not collapse the
  // instant open marks the revealed replies read. A reply absent from the
  // snapshot (loaded after open) falls back to its live marker.
  const { firstUnreadReplyId: threadFirstUnreadReplyId } = React.useMemo(() => {
    if (!openThreadHeadId || threadMessages.length === 0) {
      return { firstUnreadReplyId: null, unreadCount: 0 };
    }
    const snapshot = threadOpenReadSnapshotRef.current.get(openThreadHeadId);
    const replies = threadMessages.map((entry) => entry.message);
    return computeThreadUnreadMarker(
      replies,
      // Use the snapshot value when the reply was captured — even when it is
      // null (never read on open). Distinguish "captured null" from "never
      // captured" with `has`, not `??`: a never-read reply snapshots to null,
      // and a nullish-coalescing fallthrough would discard that and re-read the
      // now-advanced live marker, collapsing the divider over the very replies
      // that should anchor it.
      (replyId) =>
        snapshot?.has(replyId)
          ? (snapshot.get(replyId) ?? null)
          : getMessageReadAt(replyId),
      currentPubkey,
    );
  }, [currentPubkey, getMessageReadAt, openThreadHeadId, threadMessages]);
  // Per-row subtree unread counts for the in-panel thread summary rows. Scoped
  // to the open thread's subtree and decided per-reply against the live
  // per-message read state (getMessageReadAt): each collapsed row's badge
  // counts unread replies anywhere beneath it. Expanding a branch marks only
  // its revealed direct children read, so a collapsed grandchild keeps its
  // badge — the per-message marker distinguishes the read parent from the
  // unread descendant with no separate expanded-subtree gate. readStateVersion
  // is an intentional recompute trigger so the counts re-read after any marker
  // advances.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and forcedUnreadVersion are intentional recompute triggers
  const threadReplyUnreadCounts = React.useMemo(
    () =>
      openThreadHeadId
        ? computeThreadReplyUnreadCounts({
            timelineMessages,
            subtreeReplyIds: getReplyDescendantIdsForMessage(openThreadHeadId),
            visibleReplyIds: threadMessages.map((entry) => entry.message.id),
            expandedReplyIds: expandedThreadReplyIds,
            getReadAt: getMessageReadAt,
            currentPubkey,
            isForcedUnread: isMsgForcedUnread,
          })
        : new Map<string, number>(),
    [
      openThreadHeadId,
      threadMessages,
      timelineMessages,
      getMessageReadAt,
      expandedThreadReplyIds,
      getReplyDescendantIdsForMessage,
      currentPubkey,
      isMsgForcedUnread,
      readStateVersion,
      forcedUnreadVersion,
    ],
  );
  // Per-thread unread counts for the main-timeline summary rows. Unread is
  // decided per-reply against the live per-message read state: each reply
  // lights iff createdAt > effective(msg:<id>), folded channel→message only by
  // the parent resolver, so reading an ancestor never clears a descendant
  // (LP4 Issue 2 by construction). readStateVersion is an intentional recompute
  // trigger so the badge re-reads after any marker advances.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and forcedUnreadVersion are intentional recompute triggers
  const threadUnreadCountsRaw = React.useMemo(
    () =>
      computeThreadBadgeCounts(
        timelineMessages,
        repliesByRootId,
        getMessageReadAt,
        (rootId) => !isThreadMuted(rootId),
        currentPubkey,
        isMsgForcedUnread,
      ),
    [
      currentPubkey,
      timelineMessages,
      repliesByRootId,
      getMessageReadAt,
      isThreadMuted,
      isMsgForcedUnread,
      readStateVersion,
      forcedUnreadVersion,
    ],
  );
  // Keep the same Map reference across recomputes when the entries are
  // unchanged. readStateVersion bumps on read-state activity (frequent in busy
  // channels) and recomputes a fresh Map even when no count changed; without
  // this, every bump hands the timeline a new prop ref and reconciles all rows.
  const threadUnreadCounts = useStableMap(threadUnreadCountsRaw);

  // Per-message unread predicate for the mark-read/unread menu toggle. Reuses
  // computeThreadUnreadMarker — the exact function the badge counts call
  // (computeThreadBadgeCounts) — over a single-message array, so the menu label
  // and the badge can never disagree: one source of truth, no re-derived
  // predicate to drift. A message absent from the timeline (never loaded) is
  // treated as read, matching the badge, which only tallies loaded messages.
  // readStateVersion recomputes on marker advances; forcedUnreadVersion bumps
  // on every mark-read/unread so the callback identity changes and the value
  // re-flows through the memoized message subtree (forcedUnreadMsgRef is a ref,
  // invisible to React on its own). Both keep the menu label and the badge —
  // which read the same computeThreadUnreadMarker predicate — from drifting.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and forcedUnreadVersion are intentional recompute triggers
  const isMessageUnread = React.useCallback(
    (messageId: string): boolean => {
      const message = messageById.get(messageId);
      if (!message) return false;
      const { firstUnreadReplyId } = computeThreadUnreadMarker(
        [message],
        getMessageReadAt,
        currentPubkey,
        isMsgForcedUnread,
      );
      return firstUnreadReplyId !== null;
    },
    [
      messageById,
      getMessageReadAt,
      currentPubkey,
      isMsgForcedUnread,
      readStateVersion,
      forcedUnreadVersion,
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

  // Mark a message's directly-revealed children read (LP4 v3 open-at-level):
  // expanding a branch reveals only its direct replies, so only those get a
  // msg:<id> marker advanced to their createdAt. A reply still nested in a
  // collapsed grandchild branch keeps its badge until it too is revealed.
  //
  // Capture each child's pre-read state into the divider snapshot BEFORE
  // advancing its marker. This path runs synchronously in the expand event
  // handler, before React re-renders with the child visible — so without the
  // pre-capture the render-time pass above would snapshot the child as already
  // read (this mark-read having won the race) and the "New" divider would never
  // anchor to a reply first revealed by expansion.
  const markRevealedRepliesRead = React.useCallback(
    (messageId: string) => {
      for (const replyId of directReplyIdsByParentId.get(messageId) ?? []) {
        const createdAt = createdAtByMessageId.get(replyId);
        if (createdAt !== undefined) {
          captureDividerReadState(replyId);
          markMessageRead(replyId, createdAt);
        }
      }
    },
    [
      captureDividerReadState,
      createdAtByMessageId,
      directReplyIdsByParentId,
      markMessageRead,
    ],
  );

  // Mark a message and its whole subtree READ (LP4 v3 menu action). Writes a
  // msg:<id> marker at each message's createdAt — a real, persisted advance —
  // and clears those same ids from the forced-unread overlay, so mark-read is
  // the exact inverse of mark-unread over the same id set.
  const handleMarkMessageRead = React.useCallback(
    (messageId: string) => {
      const ids = [messageId, ...getReplyDescendantIdsForMessage(messageId)];
      for (const id of ids) {
        forcedUnreadMsgRef.current.delete(id);
        const createdAt = createdAtByMessageId.get(id);
        if (createdAt !== undefined) markMessageRead(id, createdAt);
      }
      forceUnreadRender();
    },
    [createdAtByMessageId, getReplyDescendantIdsForMessage, markMessageRead],
  );

  // Mark a message and its whole subtree UNREAD (LP4 v3 menu action). Markers
  // are monotonic and cannot move backward, so this writes NO marker: it adds
  // the ids to the session-local forced-unread overlay the badge predicates OR
  // in. Cleared on channel-leave; does not survive reload (symmetric with the
  // shipped channel mark-unread).
  const handleMarkMessageUnread = React.useCallback(
    (messageId: string) => {
      for (const id of [
        messageId,
        ...getReplyDescendantIdsForMessage(messageId),
      ]) {
        forcedUnreadMsgRef.current.add(id);
      }
      forceUnreadRender();
    },
    [getReplyDescendantIdsForMessage],
  );

  return {
    createdAtByMessageId,
    directReplyIdsByParentId,
    firstUnreadMessageId,
    getFirstReplyIdForMessage,
    getReplyDescendantIdsForMessage,
    handleMarkMessageRead,
    handleMarkMessageUnread,
    handleMarkUnread,
    isMessageUnread,
    markRevealedRepliesRead,
    openThreadHeadMessage,
    threadFirstUnreadReplyId,
    threadMessages,
    threadReplyTargetMessage,
    threadReplyUnreadCounts,
    threadUnreadCounts,
    unreadCount,
  };
}
