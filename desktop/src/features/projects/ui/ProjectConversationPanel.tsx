import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useChannelMembersQuery,
  useChannelsQuery,
} from "@/features/channels/hooks";
import { getThreadPanelLayout } from "@/features/channels/lib/threadPanelLayout";
import type { ThreadViewMode } from "@/features/channels/lib/threadViewModePreference";
import { BotActivityComposerAction } from "@/features/channels/ui/BotActivityBar";
import { FocusThreadDrawer } from "@/features/channels/ui/FocusThreadDrawer";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { ThreadViewModeToggle } from "@/features/channels/ui/ThreadViewModeToggle";
import { useThreadViewModeSwitch } from "@/features/channels/ui/useThreadViewModeSwitch";
import {
  useChannelSubscription,
  useDeleteMessageMutation,
  useEditMessageMutation,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { buildMessageComposerEditTarget } from "@/features/messages/lib/draftMentionRefs";
import { getRecentMentionPubkeys } from "@/features/messages/lib/recentMentionPubkeys";
import { getSendToChannelSemantics } from "@/features/messages/lib/sendToChannelSemantics";
import { summarizeThreadRoot } from "@/features/messages/lib/sentFromThread";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useIndependentThreadPanel } from "@/features/messages/useIndependentThreadPanel";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import { DeleteMessageConfirmDialog } from "@/features/messages/ui/DeleteMessageConfirmDialog";
import { MessageThreadPanel } from "@/features/messages/ui/MessageThreadPanel";
import { MessageThreadPanelSkeleton } from "@/features/messages/ui/MessageThreadPanelSkeleton";
import type { TimelineMessage } from "@/features/messages/types";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { useConversationPanelActivity } from "@/features/projects/ui/useConversationPanelActivity";
import { useThreadTitle } from "@/features/threads/hooks";
import {
  firstUnreadReplyId,
  subtreeReplies,
  unreadDescendantCounts,
} from "@/features/threads/lib/threadPaneUnread";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { SearchHit } from "@/shared/api/searchTypes";
import { getEventById } from "@/shared/api/tauri";
import type { RelayEvent, RespondToMode } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { AgentSessionProvider } from "@/shared/context/AgentSessionContext";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { Button } from "@/shared/ui/button";

const EMPTY_PERSONA_LOOKUP = new Map<string, string>();
const EMPTY_RESPOND_TO_LOOKUP = new Map<string, RespondToMode>();

async function loadConversationEvent(eventId: string) {
  return getEventById(eventId);
}

/** The event to open and, when known, its thread root. */
export type ConversationTarget = Pick<
  SearchHit,
  "channelId" | "channelName" | "eventId" | "threadRootId"
>;

/** Opens the thread as the large focus drawer instead of the side pane. */
export type ConversationFocusDrawer = {
  /** Names the scrim behind the drawer, e.g. "Back to Threads". */
  backLabel: string;
};

/** Thread unread state from the surface that opened the pane. */
export type ConversationUnread = {
  /** Replies unread when the thread opened; anchors the "New" divider. */
  atOpen: ReadonlySet<string>;
  /** Replies unread right now; drives the badges and the menu's toggle. */
  current: ReadonlySet<string>;
  /** Receives the replies a "Mark read" covers. */
  onMarkRead: (replies: TimelineMessage[]) => void;
};

/** Channel thread shown beside project details and the Threads view. */
export function ProjectConversationPanel({
  canResetWidth,
  focusDrawer,
  headerLeading,
  hit,
  onClose,
  onMarkUnread,
  onResetWidth,
  onResizeStart,
  onThreadViewModeChange,
  sharedHeaderBackdrop,
  titleOpensThread = false,
  unread,
  widthPx,
}: {
  canResetWidth: boolean;
  /** Ignored while the window is narrow enough for the floating overlay. */
  focusDrawer?: ConversationFocusDrawer;
  headerLeading?: React.ReactNode;
  hit: ConversationTarget;
  onClose: () => void;
  /**
   * Receives the reply ids a "Mark unread" covers: the chosen message's
   * replies and, unless it is the root, the message itself.
   */
  onMarkUnread?: (replyIds: string[]) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  /**
   * Shows the Focus/Split toggle in the header (replacing `headerLeading`)
   * and keeps the reply being read in view across the switch, as channels do.
   * Called after the preference changes.
   */
  onThreadViewModeChange?: (mode: ThreadViewMode) => void;
  sharedHeaderBackdrop?: boolean;
  /**
   * The header shows the thread's title and opens the thread in its channel,
   * like the Inbox header, instead of linking the channel with an editor.
   */
  titleOpensThread?: boolean;
  unread?: ConversationUnread;
  widthPx: number;
}) {
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const activeChannel =
    channelsQuery.data?.find((channel) => channel.id === hit.channelId) ?? null;
  const channelId = activeChannel?.id ?? hit.channelId ?? null;
  const channelLabel = activeChannel?.name ?? hit.channelName ?? null;
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const membersQuery = useChannelMembersQuery(activeChannel?.id ?? null);
  const targetQuery = useQuery({
    queryKey: ["project-conversation-target", hit.eventId],
    queryFn: () => loadConversationEvent(hit.eventId),
  });
  const targetEvent = targetQuery.data ?? null;
  const targetThread = targetEvent
    ? getThreadReference(targetEvent.tags)
    : null;
  const rootId =
    hit.threadRootId ??
    targetThread?.rootId ??
    targetThread?.parentId ??
    targetEvent?.id ??
    null;
  const rootQuery = useQuery({
    queryKey: ["project-conversation-root", rootId ?? "none"],
    queryFn: () =>
      rootId ? loadConversationEvent(rootId) : Promise.resolve(null),
    enabled: Boolean(rootId && rootId !== targetEvent?.id),
  });
  const threadRepliesQuery = useThreadReplies(activeChannel, rootId);
  const repliesQueryEnabled =
    activeChannel != null &&
    activeChannel.channelType !== "forum" &&
    rootId != null;
  useChannelSubscription(activeChannel);
  const threadReplyEvents = threadRepliesQuery.data ?? [];
  const rootEvent =
    rootId && targetEvent?.id === rootId
      ? targetEvent
      : (rootQuery.data ?? null);
  const rootEvents = React.useMemo(
    () => (rootEvent ? [rootEvent] : []),
    [rootEvent],
  );
  const profilePubkeys = React.useMemo(
    () => [
      ...new Set(
        [rootEvent, ...threadReplyEvents]
          .filter((event): event is RelayEvent => event !== null)
          .map((event) => event.pubkey),
      ),
    ],
    [rootEvent, threadReplyEvents],
  );
  const profilesQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const [expandedReplyIds, setExpandedReplyIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const expansionKey = rootId ? `${hit.eventId}:${rootId}` : null;
  const [appliedExpansionKey, setAppliedExpansionKey] = React.useState<
    string | null
  >(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(
    rootId,
  );
  const [scrollTargetId, setScrollTargetId] = React.useState<string | null>(
    hit.eventId === rootId ? null : hit.eventId,
  );
  const clearScrollTarget = React.useCallback(
    () => setScrollTargetId(null),
    [],
  );
  const [editTargetId, setEditTargetId] = React.useState<string | null>(null);
  const [emptyDeleteId, setEmptyDeleteId] = React.useState<string | null>(null);
  const threadIdentity = `${hit.eventId}:${rootId ?? ""}`;
  const threadIdentityRef = React.useRef(threadIdentity);
  if (threadIdentityRef.current !== threadIdentity) {
    threadIdentityRef.current = threadIdentity;
    setAppliedExpansionKey(null);
    setExpandedReplyIds(new Set());
    setReplyTargetId(rootId);
    setScrollTargetId(hit.eventId === rootId ? null : hit.eventId);
    setEditTargetId(null);
    setEmptyDeleteId(null);
  }
  if (
    expansionKey &&
    repliesQueryEnabled &&
    threadRepliesQuery.isFetched &&
    appliedExpansionKey !== expansionKey
  ) {
    setAppliedExpansionKey(expansionKey);
    setExpandedReplyIds(new Set(threadReplyEvents.map((event) => event.id)));
  }

  const panelData = useIndependentThreadPanel({
    activeChannel,
    channelEvents: rootEvents,
    threadReplyEvents,
    rootId,
    replyTargetId,
    expandedReplyIds,
    currentPubkey: identityQuery.data?.pubkey,
    currentAvatarUrl: profileQuery.data?.avatarUrl ?? null,
    profiles,
    ownerProfiles: profiles,
    members: membersQuery.data,
    personaLookup: EMPTY_PERSONA_LOOKUP,
    respondToLookup: EMPTY_RESPOND_TO_LOOKUP,
    relaySelfPubkey: null,
  });
  const sendMessageMutation = useSendMessageMutation(
    activeChannel,
    identityQuery.data,
  );
  const toggleReactionMutation = useToggleReactionMutation();
  const deleteMessageMutation = useDeleteMessageMutation(activeChannel);
  const editMessageMutation = useEditMessageMutation(activeChannel);
  const { followThread, isNotifiedForThread, unfollowThread } = useAppShell();
  const isOverlay = useIsThreadPanelOverlay();
  const isComposerDisabled =
    !activeChannel?.isMember ||
    activeChannel.archivedAt !== null ||
    activeChannel.channelType === "forum";
  const canManageMessages = Boolean(activeChannel && !activeChannel.archivedAt);
  const editTargetMessage = editTargetId
    ? (panelData.messages.find((message) => message.id === editTargetId) ??
      null)
    : null;
  const editTarget = editTargetMessage
    ? buildMessageComposerEditTarget(
        editTargetMessage,
        profiles,
        (pubkey) => !!profiles?.[pubkey]?.isAgent,
      )
    : null;
  const deleteMessage = React.useCallback(
    async (message: { id: string }) => {
      try {
        await deleteMessageMutation.mutateAsync({ eventId: message.id });
      } catch {
        // Failure is surfaced via the mutation's onError toast.
        return;
      }
      // The root is loaded on its own, outside the replies cache, so a
      // deleted root would linger; there is no thread left to show.
      if (message.id === rootId) onClose();
    },
    [deleteMessageMutation.mutateAsync, onClose, rootId],
  );
  const handleEdit = React.useCallback((message: { id: string }) => {
    setEditTargetId((current) => (current === message.id ? null : message.id));
  }, []);
  const currentPubkey = identityQuery.data?.pubkey;
  // ArrowUp in an empty composer edits your newest visible message, as the
  // channel's thread panel does (useRoutedMessageEdit).
  const handleEditLastOwnMessage = React.useCallback(() => {
    if (!currentPubkey || !panelData.threadHead) return false;
    let target: TimelineMessage | null = null;
    for (const message of [
      panelData.threadHead,
      ...panelData.visibleReplies.map((entry) => entry.message),
    ]) {
      if (
        message.kind === KIND_SYSTEM_MESSAGE ||
        message.pubkey !== currentPubkey ||
        message.pending ||
        (target && message.createdAt < target.createdAt)
      )
        continue;
      target = message;
    }
    if (!target) return false;
    setEditTargetId(target.id);
    return true;
  }, [currentPubkey, panelData.threadHead, panelData.visibleReplies]);
  const handleEditSave = React.useCallback(
    async (
      content: string,
      mediaTags?: string[][],
      mentionPubkeys?: string[],
    ) => {
      if (!editTargetId) return;
      // Clearing an edit to empty means "Delete message", behind the same
      // confirmation the menu's Delete shows (as in the channel view).
      if (content.trim().length === 0 && !mediaTags?.length) {
        setEmptyDeleteId(editTargetId);
        return;
      }
      await editMessageMutation.mutateAsync({
        eventId: editTargetId,
        content,
        mediaTags,
        mentionPubkeys,
      });
      setEditTargetId((current) => (current === editTargetId ? null : current));
    },
    [editMessageMutation.mutateAsync, editTargetId],
  );
  const handleSendToChannel = React.useCallback(
    async (
      message: TimelineMessage,
      threadRoot: TimelineMessage,
      targetChannelId: string,
    ) => {
      const { mentionPubkeys, semanticTags } = getSendToChannelSemantics(
        message,
        profiles,
      );
      await sendMessageMutation.mutateAsync({
        channelId: targetChannelId,
        content: message.body,
        mediaTags: semanticTags,
        mentionPubkeys,
        sentFromThreadRootExcerpt: summarizeThreadRoot(threadRoot.body),
        sentFromThreadRootId: threadRoot.id,
      });
    },
    [profiles, sendMessageMutation.mutateAsync],
  );
  const handleMarkUnread = React.useCallback(
    (message: TimelineMessage) => {
      if (!onMarkUnread) return;
      onMarkUnread(
        subtreeReplies(message.id, rootId, panelData.messages).map(
          (reply) => reply.id,
        ),
      );
    },
    [onMarkUnread, panelData.messages, rootId],
  );
  const onMarkRead = unread?.onMarkRead;
  const handleMarkRead = React.useCallback(
    (message: TimelineMessage) => {
      onMarkRead?.(subtreeReplies(message.id, rootId, panelData.messages));
    },
    [onMarkRead, panelData.messages, rootId],
  );
  const unreadMarkers = React.useMemo(() => {
    if (!unread) return null;
    const replies = panelData.messages.filter(
      (message) => message.id !== rootId,
    );
    return {
      firstUnreadReplyId: firstUnreadReplyId(
        panelData.visibleReplies.map((entry) => entry.message),
        unread.atOpen,
        currentPubkey,
      ),
      threadUnreadCount: replies.filter((reply) => unread.current.has(reply.id))
        .length,
      threadReplyUnreadCounts: unreadDescendantCounts(
        panelData.messages,
        unread.current,
      ),
    };
  }, [
    currentPubkey,
    panelData.messages,
    panelData.visibleReplies,
    rootId,
    unread,
  ]);
  const unreadCurrent = unread?.current;
  const isMessageUnreadById = React.useCallback(
    (messageId: string) => unreadCurrent?.has(messageId) ?? false,
    [unreadCurrent],
  );
  const recentMentionPubkeys = React.useMemo(
    () =>
      getRecentMentionPubkeys(
        [...panelData.messages].sort((a, b) => a.createdAt - b.createdAt),
        activeChannel?.channelType,
      ),
    [activeChannel?.channelType, panelData.messages],
  );
  const conversationEvents = React.useMemo(
    () => (rootEvent ? [rootEvent, ...threadReplyEvents] : threadReplyEvents),
    [rootEvent, threadReplyEvents],
  );
  const activity = useConversationPanelActivity({
    activeChannel,
    channelMembers: membersQuery.data,
    currentPubkey,
    events: conversationEvents,
    rootId,
  });
  const openAgentSession = React.useCallback(
    (pubkey: string, sessionChannelId?: string | null) => {
      const targetChannelId = sessionChannelId ?? channelId;
      if (targetChannelId) {
        void goChannel(targetChannelId, { agentSession: pubkey });
      }
    },
    [channelId, goChannel],
  );
  const { changeThreadViewMode, layoutScrollTargetId, resolveScrollTarget } =
    useThreadViewModeSwitch({
      activeThreadHeadId: rootId,
      externalScrollTargetId: scrollTargetId,
      onExternalTargetResolved: clearScrollTarget,
      onModeChange: onThreadViewModeChange,
    });
  const panelHeaderLeading =
    onThreadViewModeChange && !isOverlay ? (
      <ThreadViewModeToggle onChange={changeThreadViewMode} />
    ) : (
      headerLeading
    );
  const isFollowingThread = rootId ? isNotifiedForThread(rootId) : false;
  const canShowThread =
    Boolean(activeChannel && panelData.threadHead) &&
    (expansionKey == null ||
      !repliesQueryEnabled ||
      appliedExpansionKey === expansionKey);
  const isResolving =
    !canShowThread &&
    (channelsQuery.isPending ||
      targetQuery.isPending ||
      Boolean(
        rootId &&
          targetEvent &&
          rootId !== targetEvent.id &&
          rootQuery.isPending,
      ) ||
      Boolean(repliesQueryEnabled && !threadRepliesQuery.isFetched) ||
      Boolean(
        repliesQueryEnabled &&
          expansionKey &&
          threadRepliesQuery.isFetched &&
          appliedExpansionKey !== expansionKey,
      ));
  const openChannel = React.useCallback(() => {
    if (channelId) void goChannel(channelId);
  }, [channelId, goChannel]);
  const sharedThreadTitle = useThreadTitle(channelId, rootId)?.title ?? null;
  const openThreadInChannel = React.useCallback(() => {
    if (channelId && rootId) {
      void goChannel(channelId, { messageId: rootId, threadRootId: rootId });
    }
  }, [channelId, goChannel, rootId]);
  const threadLinkTitle = sharedThreadTitle
    ? channelLabel
      ? `${sharedThreadTitle} · #${channelLabel}`
      : sharedThreadTitle
    : channelLabel
      ? `Thread in #${channelLabel}`
      : "Thread";
  const activeFocusDrawer = isOverlay ? undefined : focusDrawer;
  const layoutProps = {
    canResetWidth,
    enterMotion: !canShowThread,
    headerLeading: panelHeaderLeading,
    ...(titleOpensThread
      ? {
          headerTitle: threadLinkTitle,
          headerTitleAriaLabel: "Open full thread",
          onHeaderTitleClick:
            channelId && rootId ? openThreadInChannel : undefined,
          threadTitleEditable: false,
        }
      : {
          headerTitle: channelLabel ? `#${channelLabel}` : "Thread",
          headerTitleAriaLabel: channelLabel
            ? `Open #${channelLabel}`
            : undefined,
          onHeaderTitleClick: channelId ? openChannel : undefined,
        }),
    isFocusMode: false,
    isSinglePanelView: !isOverlay,
    layout: "standalone" as const,
    onResetWidth,
    onResizeStart,
    showBackButton: false,
    splitPaneClamp: false,
    testId: isOverlay ? "project-conversation-panel" : "message-thread-panel",
    transparentChrome: sharedHeaderBackdrop,
    ...(activeFocusDrawer
      ? getThreadPanelLayout({
          headerLeading: panelHeaderLeading,
          isFocusDrawer: true,
          isSinglePanelView: true,
          useSplitAuxiliaryPane: true,
        })
      : null),
  };
  const paneOptions = {
    canResetWidth,
    channelName: activeChannel?.name ?? hit.channelName ?? "channel",
    focusDrawer: activeFocusDrawer,
    isOverlay,
    onClose,
    onResetWidth,
    onResizeStart,
    widthPx,
  };

  const handleSend = React.useCallback(
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
      if (!activeChannel || !rootId) return;
      const parentEventId =
        threadContext?.parentEventId ?? replyTargetId ?? rootId;
      const sentMessage = await sendMessageMutation.mutateAsync({
        channelId: channelId ?? activeChannel.id,
        content,
        mediaTags,
        mentionPubkeys,
        parentEventId,
        targetChannel: activeChannel,
      });
      setReplyTargetId(rootId);
      setExpandedReplyIds((current) => new Set(current).add(parentEventId));
      setScrollTargetId(sentMessage.id);
    },
    [activeChannel, replyTargetId, rootId, sendMessageMutation.mutateAsync],
  );
  const handleExpandReplies = React.useCallback((message: TimelineMessage) => {
    setExpandedReplyIds((current) => {
      const next = new Set(current);
      if (next.has(message.id)) next.delete(message.id);
      else next.add(message.id);
      return next;
    });
  }, []);

  if (canShowThread && activeChannel && panelData.threadHead) {
    return wrapProjectConversationPane(
      <AgentSessionProvider onOpenAgentSession={openAgentSession}>
        <MessageThreadPanel
          {...layoutProps}
          activityAccessoryContent={
            activity.workingAgentPubkeys.length > 0 ? (
              <BotActivityComposerAction
                agents={activity.agents}
                channelId={activeChannel.id}
                onOpenAgentSession={openAgentSession}
                openAgentSessionPubkey={null}
                profiles={profiles}
                variant="inline"
                workingBotPubkeys={activity.workingAgentPubkeys}
              />
            ) : null
          }
          activityAccessoryVisible={activity.workingAgentPubkeys.length > 0}
          channel={activeChannel}
          channelId={activeChannel.id}
          channelName={activeChannel.name}
          currentPubkey={identityQuery.data?.pubkey}
          disabled={isComposerDisabled}
          editTarget={editTarget}
          firstUnreadReplyId={unreadMarkers?.firstUnreadReplyId}
          isFollowingThread={isFollowingThread}
          isMessageUnreadById={unread ? isMessageUnreadById : undefined}
          isSending={sendMessageMutation.isPending}
          onCancelEdit={() => setEditTargetId(null)}
          onCancelReply={() => setReplyTargetId(rootId)}
          onClose={onClose}
          onDelete={canManageMessages ? deleteMessage : undefined}
          onEdit={canManageMessages ? handleEdit : undefined}
          onEditSave={canManageMessages ? handleEditSave : undefined}
          onEditLastOwnMessage={
            canManageMessages ? handleEditLastOwnMessage : undefined
          }
          onExpandReplies={handleExpandReplies}
          onMarkRead={unread ? handleMarkRead : undefined}
          onMarkUnread={onMarkUnread ? handleMarkUnread : undefined}
          onFollowThread={
            rootId && !isFollowingThread
              ? () => followThread(rootId)
              : undefined
          }
          onUnfollowThread={
            rootId && isFollowingThread
              ? () => unfollowThread(rootId)
              : undefined
          }
          onSendToChannel={isComposerDisabled ? undefined : handleSendToChannel}
          onScrollTargetResolved={() => resolveScrollTarget()}
          onScrollTargetSettled={resolveScrollTarget}
          onSelectReplyTarget={(message) => setReplyTargetId(message.id)}
          onSend={handleSend}
          onToggleReaction={
            isComposerDisabled
              ? undefined
              : async (message, emoji, remove) => {
                  await toggleReactionMutation.mutateAsync({
                    emoji,
                    eventId: message.id,
                    remove,
                  });
                }
          }
          profiles={profiles}
          recentMentionPubkeys={recentMentionPubkeys}
          replyTargetMessage={panelData.replyTargetMessage}
          scrollTargetHighlights={!layoutScrollTargetId}
          scrollTargetId={layoutScrollTargetId ?? scrollTargetId}
          threadHead={panelData.threadHead}
          threadReplies={panelData.visibleReplies}
          threadRepliesPending={threadRepliesQuery.isPending}
          threadRepliesError={threadRepliesQuery.isError}
          onRetryThreadReplies={() => {
            void threadRepliesQuery.refetch();
          }}
          threadReplyUnreadCounts={unreadMarkers?.threadReplyUnreadCounts}
          threadTypingPubkeys={activity.threadTypingPubkeys}
          threadUnreadCount={unreadMarkers?.threadUnreadCount}
          widthPx={widthPx}
        />
        <DeleteMessageConfirmDialog
          onConfirm={() => {
            if (emptyDeleteId) {
              setEditTargetId(null);
              void deleteMessage({ id: emptyDeleteId });
            }
            setEmptyDeleteId(null);
          }}
          onOpenChange={(open) => {
            if (!open) setEmptyDeleteId(null);
          }}
          open={emptyDeleteId !== null}
        />
      </AgentSessionProvider>,
      paneOptions,
    );
  }

  if (isResolving) {
    return wrapProjectConversationPane(
      <MessageThreadPanelSkeleton
        {...layoutProps}
        onClose={onClose}
        widthPx={widthPx}
      />,
      paneOptions,
    );
  }

  return wrapProjectConversationPane(
    <div className="flex h-full flex-col items-center justify-center gap-3 bg-background p-6 text-center">
      <p className="text-sm text-muted-foreground">
        This conversation could not be loaded.
      </p>
      <Button onClick={onClose} size="sm" variant="outline">
        Close
      </Button>
    </div>,
    paneOptions,
  );
}

function wrapProjectConversationPane(
  panel: React.ReactNode,
  {
    canResetWidth,
    channelName,
    focusDrawer,
    isOverlay,
    onClose,
    onResetWidth,
    onResizeStart,
    widthPx,
  }: {
    canResetWidth: boolean;
    channelName: string;
    focusDrawer: ConversationFocusDrawer | undefined;
    isOverlay: boolean;
    onClose: () => void;
    onResetWidth: () => void;
    onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
    widthPx: number;
  },
) {
  if (isOverlay) return panel;
  if (focusDrawer) {
    return (
      <FocusThreadDrawer
        backLabel={focusDrawer.backLabel}
        channelName={channelName}
        onClose={onClose}
      >
        {panel}
      </FocusThreadDrawer>
    );
  }
  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId="project-conversation-panel"
      widthPx={widthPx}
    >
      <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden [&>aside]:!h-full [&>aside]:!w-full [&>aside]:!max-w-none [&>aside]:!border-l-0">
        {panel}
      </div>
    </RightAuxiliaryPane>
  );
}
