import * as React from "react";

import type {
  useDeleteMessageMutation,
  useEditMessageMutation,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { resolveThreadReplyTarget } from "@/features/messages/hooks";

/**
 * Stable callback references for ChannelPane so that keystroke-driven
 * re-renders of ChannelScreen don't cascade into the timeline and composer.
 *
 * Mutation objects from TanStack Query v5 are new references on every render
 * (especially when `isPending` flips), so we stash `.mutateAsync` in a ref
 * rather than listing the whole mutation as a dependency.
 */
export function useChannelPaneHandlers({
  deleteMessageMutation,
  editMessageMutation,
  editTargetId,
  expandedThreadReplyIds,
  getFirstReplyIdForMessage,
  getReplyDescendantIdsForMessage,
  markRevealedRepliesRead,
  onOptimisticOpenThreadHeadIdChange,
  openThreadHeadId,
  sendMessageMutation,
  setExpandedThreadReplyIds,
  setEditTargetId,
  setOpenThreadHeadId,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
  threadReplyTargetId,
  toggleReactionMutation,
}: {
  deleteMessageMutation: ReturnType<typeof useDeleteMessageMutation>;
  editMessageMutation: ReturnType<typeof useEditMessageMutation>;
  editTargetId: string | null;
  expandedThreadReplyIds: ReadonlySet<string>;
  getFirstReplyIdForMessage: (messageId: string) => string | null;
  getReplyDescendantIdsForMessage: (messageId: string) => string[];
  markRevealedRepliesRead: (messageId: string) => void;
  onOptimisticOpenThreadHeadIdChange: React.Dispatch<
    React.SetStateAction<string | null | undefined>
  >;
  openThreadHeadId: string | null;
  sendMessageMutation: ReturnType<typeof useSendMessageMutation>;
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setEditTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  setOpenThreadHeadId: (value: string | null) => void;
  setThreadReplyTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  setThreadScrollTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  threadReplyTargetId: string | null;
  toggleReactionMutation: ReturnType<typeof useToggleReactionMutation>;
}) {
  // Keep mutable values in refs so callbacks never need to list them as deps.
  const openThreadHeadIdRef = React.useRef(openThreadHeadId);
  openThreadHeadIdRef.current = openThreadHeadId;

  const threadReplyTargetIdRef = React.useRef(threadReplyTargetId);
  threadReplyTargetIdRef.current = threadReplyTargetId;

  const editTargetIdRef = React.useRef(editTargetId);
  editTargetIdRef.current = editTargetId;

  const expandedThreadReplyIdsRef = React.useRef(expandedThreadReplyIds);
  expandedThreadReplyIdsRef.current = expandedThreadReplyIds;

  const sendMutateRef = React.useRef(sendMessageMutation.mutateAsync);
  sendMutateRef.current = sendMessageMutation.mutateAsync;

  const deleteMutateRef = React.useRef(deleteMessageMutation.mutateAsync);
  deleteMutateRef.current = deleteMessageMutation.mutateAsync;

  const editMutateRef = React.useRef(editMessageMutation.mutateAsync);
  editMutateRef.current = editMessageMutation.mutateAsync;

  const toggleMutateRef = React.useRef(toggleReactionMutation.mutateAsync);
  toggleMutateRef.current = toggleReactionMutation.mutateAsync;

  // These three recompute whenever timelineMessages changes (every ingest).
  // Read them through refs so handleExpandThreadReplies keeps a stable
  // identity — it feeds MessageThreadPanel's per-row onCollapseDepthGuide,
  // and an identity change there re-renders every thread row. With agents
  // streaming into an open long thread that meant all rows re-rendered
  // several times per second (see typing-latency.perf.ts "thread68+").
  const getFirstReplyIdRef = React.useRef(getFirstReplyIdForMessage);
  getFirstReplyIdRef.current = getFirstReplyIdForMessage;

  const getReplyDescendantIdsRef = React.useRef(
    getReplyDescendantIdsForMessage,
  );
  getReplyDescendantIdsRef.current = getReplyDescendantIdsForMessage;

  const markRevealedRepliesReadRef = React.useRef(markRevealedRepliesRead);
  markRevealedRepliesReadRef.current = markRevealedRepliesRead;

  const deferPanelState = React.useCallback((update: () => void) => {
    window.setTimeout(() => {
      React.startTransition(update);
    }, 0);
  }, []);

  const handleCancelThreadReply = React.useCallback(() => {
    setThreadReplyTargetId(openThreadHeadIdRef.current);
  }, [setThreadReplyTargetId]);

  const handleCloseThread = React.useCallback(() => {
    deferPanelState(() => {
      onOptimisticOpenThreadHeadIdChange(null);
      setOpenThreadHeadId(null);
      setThreadReplyTargetId(null);
      setThreadScrollTargetId(null);
      setExpandedThreadReplyIds(new Set());
    });
  }, [
    deferPanelState,
    onOptimisticOpenThreadHeadIdChange,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);

  const handleCancelEdit = React.useCallback(() => {
    setEditTargetId(null);
  }, [setEditTargetId]);

  const handleDelete = React.useCallback(async (message: { id: string }) => {
    // Failure is surfaced via the mutation's onError toast.
    await deleteMutateRef.current({ eventId: message.id }).catch(() => {});
  }, []);

  const handleEdit = React.useCallback(
    (message: { id: string }) => {
      setEditTargetId((current) =>
        current === message.id ? null : message.id,
      );
      setThreadReplyTargetId(openThreadHeadIdRef.current);
    },
    [setEditTargetId, setThreadReplyTargetId],
  );

  const handleEditSave = React.useCallback(
    async (
      content: string,
      mediaTags?: string[][],
      mentionPubkeys?: string[],
    ) => {
      const eventId = editTargetIdRef.current;
      if (!eventId) {
        return;
      }

      await editMutateRef.current({
        eventId,
        content,
        mediaTags,
        mentionPubkeys,
      });
      setEditTargetId(null);
    },
    [setEditTargetId],
  );

  const handleOpenThread = React.useCallback(
    (message: { id: string }) => {
      if (openThreadHeadIdRef.current === message.id) {
        deferPanelState(() => {
          onOptimisticOpenThreadHeadIdChange(null);
          setOpenThreadHeadId(null);
          setThreadReplyTargetId(null);
          setThreadScrollTargetId(null);
          setExpandedThreadReplyIds(new Set());
        });
        setEditTargetId(null);
        return;
      }

      deferPanelState(() => {
        onOptimisticOpenThreadHeadIdChange(message.id);
        setOpenThreadHeadId(message.id);
        setThreadReplyTargetId(message.id);
        setThreadScrollTargetId(null);
        setExpandedThreadReplyIds(new Set());
      });
      setEditTargetId(null);
    },
    [
      deferPanelState,
      onOptimisticOpenThreadHeadIdChange,
      setEditTargetId,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const handleSelectThreadReplyTarget = React.useCallback(
    (message: { id: string }) => {
      if (threadReplyTargetIdRef.current === message.id) {
        setThreadReplyTargetId(openThreadHeadIdRef.current);
      } else {
        setThreadReplyTargetId(message.id);
      }
      setEditTargetId(null);
    },
    [setEditTargetId, setThreadReplyTargetId],
  );

  const handleExpandThreadReplies = React.useCallback(
    (message: { id: string }) => {
      if (expandedThreadReplyIdsRef.current.has(message.id)) {
        const descendantIds = getReplyDescendantIdsRef.current(message.id);
        setExpandedThreadReplyIds((current) => {
          const next = new Set(current);
          next.delete(message.id);
          for (const descendantId of descendantIds) {
            next.delete(descendantId);
          }
          return next;
        });
        return;
      }

      const firstReplyId = getFirstReplyIdRef.current(message.id);
      setExpandedThreadReplyIds((current) => {
        const next = new Set(current);
        next.add(message.id);
        return next;
      });

      // Drilling into a branch reveals only its direct replies (LP4 v3
      // open-at-level): mark exactly those read, never the whole subtree. A
      // reply still nested in a collapsed grandchild branch keeps its badge
      // until it too is revealed — the deliberate reversal of #1118's
      // whole-subtree-on-open collapse.
      markRevealedRepliesReadRef.current(message.id);

      if (firstReplyId) {
        setThreadScrollTargetId(firstReplyId);
      }
    },
    [setExpandedThreadReplyIds, setThreadScrollTargetId],
  );

  const handleSendMessage = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      channelId?: string | null,
    ) => {
      await sendMutateRef.current({
        content,
        mentionPubkeys,
        mediaTags,
        channelId: channelId ?? undefined,
      });
    },
    [],
  );

  const handleSendThreadReply = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      channelId?: string | null,
      threadContext?: {
        parentEventId: string | null;
        threadHeadId: string | null;
      } | null,
    ) => {
      // Resolve target using captured submit-time context (race-free) or live
      // refs (legacy path). When threadContext is supplied, no live-ref reads
      // occur after the mention-flow awaits; the resolution is purely data.
      const target = resolveThreadReplyTarget(
        threadContext,
        threadReplyTargetIdRef.current,
        openThreadHeadIdRef.current,
      );
      if (!target) {
        return;
      }
      const { parentEventId, threadHeadId: activeThreadHeadId } = target;

      if (
        activeThreadHeadId &&
        parentEventId !== activeThreadHeadId &&
        !expandedThreadReplyIdsRef.current.has(parentEventId)
      ) {
        setExpandedThreadReplyIds((current) => {
          const next = new Set(current);
          next.add(parentEventId);
          return next;
        });
      }

      const sentMessage = await sendMutateRef.current({
        content,
        mentionPubkeys,
        parentEventId,
        mediaTags,
        channelId: channelId ?? undefined,
      });

      // Only update thread UI state if the user is still viewing the same
      // thread. If they navigated away during the async send, don't disrupt
      // the thread they are currently viewing.
      if (openThreadHeadIdRef.current === activeThreadHeadId) {
        setThreadReplyTargetId(activeThreadHeadId);
        if (activeThreadHeadId && parentEventId !== activeThreadHeadId) {
          setThreadScrollTargetId(sentMessage.id);
        }
      }
    },
    [
      setExpandedThreadReplyIds,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const handleToggleReaction = React.useCallback(
    async (message: { id: string }, emoji: string, remove: boolean) => {
      await toggleMutateRef.current({
        emoji,
        eventId: message.id,
        remove,
      });
    },
    [],
  );

  return {
    handleCancelEdit,
    handleCancelThreadReply,
    handleCloseThread,
    handleDelete,
    handleEdit,
    handleEditSave,
    handleExpandThreadReplies,
    handleOpenThread,
    handleSendMessage,
    handleSendThreadReply,
    handleSelectThreadReplyTarget,
    handleToggleReaction,
  };
}
