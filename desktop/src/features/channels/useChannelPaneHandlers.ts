import * as React from "react";

import { flushSync } from "react-dom";

import type {
  useDeleteMessageMutation,
  useEditMessageMutation,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { mergeAutoRouteMentionPubkeys } from "@/features/channels/ui/ChannelPane.helpers";

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
  messageAutoRouteAgentPubkeys,
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
  threadAutoRouteAgentPubkeys,
  threadReplyTargetId,
  toggleReactionMutation,
}: {
  deleteMessageMutation: ReturnType<typeof useDeleteMessageMutation>;
  editMessageMutation: ReturnType<typeof useEditMessageMutation>;
  editTargetId: string | null;
  messageAutoRouteAgentPubkeys: readonly string[];
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
  threadAutoRouteAgentPubkeys: readonly string[];
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

  const messageAutoRouteAgentPubkeysRef = React.useRef(
    messageAutoRouteAgentPubkeys,
  );
  messageAutoRouteAgentPubkeysRef.current = messageAutoRouteAgentPubkeys;

  const threadAutoRouteAgentPubkeysRef = React.useRef(
    threadAutoRouteAgentPubkeys,
  );
  threadAutoRouteAgentPubkeysRef.current = threadAutoRouteAgentPubkeys;

  const sendMutateRef = React.useRef(sendMessageMutation.mutateAsync);
  sendMutateRef.current = sendMessageMutation.mutateAsync;

  const deleteMutateRef = React.useRef(deleteMessageMutation.mutateAsync);
  deleteMutateRef.current = deleteMessageMutation.mutateAsync;

  const editMutateRef = React.useRef(editMessageMutation.mutateAsync);
  editMutateRef.current = editMessageMutation.mutateAsync;

  const toggleMutateRef = React.useRef(toggleReactionMutation.mutateAsync);
  toggleMutateRef.current = toggleReactionMutation.mutateAsync;

  const handleCancelThreadReply = React.useCallback(() => {
    setThreadReplyTargetId(openThreadHeadIdRef.current);
  }, [setThreadReplyTargetId]);

  const handleCloseThread = React.useCallback(() => {
    flushSync(() => {
      onOptimisticOpenThreadHeadIdChange(null);
    });
    setOpenThreadHeadId(null);
    setThreadReplyTargetId(null);
    setThreadScrollTargetId(null);
    setExpandedThreadReplyIds(new Set());
  }, [
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
    await deleteMutateRef.current({ eventId: message.id });
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
    async (content: string, mediaTags?: string[][]) => {
      const eventId = editTargetIdRef.current;
      if (!eventId) {
        return;
      }

      await editMutateRef.current({ eventId, content, mediaTags });
      setEditTargetId(null);
    },
    [setEditTargetId],
  );

  const handleOpenThread = React.useCallback(
    (message: { id: string }) => {
      if (openThreadHeadIdRef.current === message.id) {
        flushSync(() => {
          onOptimisticOpenThreadHeadIdChange(null);
        });
        setOpenThreadHeadId(null);
        setThreadReplyTargetId(null);
        setThreadScrollTargetId(null);
        setExpandedThreadReplyIds(new Set());
        setEditTargetId(null);
        return;
      }

      flushSync(() => {
        onOptimisticOpenThreadHeadIdChange(message.id);
      });
      setOpenThreadHeadId(message.id);
      setThreadReplyTargetId(message.id);
      setThreadScrollTargetId(null);
      setExpandedThreadReplyIds(new Set());
      setEditTargetId(null);
    },
    [
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
        const descendantIds = getReplyDescendantIdsForMessage(message.id);
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

      const firstReplyId = getFirstReplyIdForMessage(message.id);
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
      markRevealedRepliesRead(message.id);

      if (firstReplyId) {
        setThreadScrollTargetId(firstReplyId);
      }
    },
    [
      getFirstReplyIdForMessage,
      getReplyDescendantIdsForMessage,
      markRevealedRepliesRead,
      setExpandedThreadReplyIds,
      setThreadScrollTargetId,
    ],
  );

  const handleSendMessage = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      await sendMutateRef.current({
        content,
        mentionPubkeys: mergeAutoRouteMentionPubkeys({
          autoRouteAgentPubkeys: messageAutoRouteAgentPubkeysRef.current,
          mentionPubkeys,
        }),
        mediaTags,
      });
    },
    [],
  );

  const handleSendThreadReply = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const activeThreadHeadId = openThreadHeadIdRef.current;
      const parentEventId =
        threadReplyTargetIdRef.current ?? activeThreadHeadId;
      if (!parentEventId) {
        return;
      }

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
        mentionPubkeys: mergeAutoRouteMentionPubkeys({
          autoRouteAgentPubkeys: threadAutoRouteAgentPubkeysRef.current,
          mentionPubkeys,
        }),
        parentEventId,
        mediaTags,
      });
      setThreadReplyTargetId(activeThreadHeadId);
      if (activeThreadHeadId && parentEventId !== activeThreadHeadId) {
        setThreadScrollTargetId(sentMessage.id);
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
